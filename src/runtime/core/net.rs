use super::*;
use std::io::{Read, Write};
use std::net::ToSocketAddrs;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;
use std::collections::HashSet;
use once_cell::sync::Lazy;
use mio::net::{TcpListener, TcpStream};
use native_tls::{TlsConnector, TlsStream};

use std::sync::RwLock;

const NUM_STREAM_SHARDS: usize = 64;
static STREAM_SHARDS: Lazy<[RwLock<HashSet<usize>>; NUM_STREAM_SHARDS]> = Lazy::new(|| {
    std::array::from_fn(|_| RwLock::new(HashSet::new()))
});
static LIVE_LISTENERS: Lazy<Mutex<HashSet<usize>>> = Lazy::new(|| Mutex::new(HashSet::new()));

#[inline(always)]
fn stream_shard(addr: usize) -> &'static RwLock<HashSet<usize>> {
    let hash = (addr ^ (addr >> 6)) % NUM_STREAM_SHARDS;
    &STREAM_SHARDS[hash]
}

fn register_stream_ptr(ptr: *mut NetStream) -> i64 {
    let addr = ptr as usize;
    if let Ok(mut set) = stream_shard(addr).write() {
        set.insert(addr);
    }
    addr as i64
}

fn register_listener_ptr(ptr: *mut NetListener) -> i64 {
    let addr = ptr as usize;
    if let Ok(mut set) = LIVE_LISTENERS.lock() {
        set.insert(addr);
    }
    addr as i64
}

fn validate_stream_ptr(stream: i64) -> Option<*mut NetStream> {
    if stream <= 0 { return None; }
    let addr = stream as usize;
    if let Ok(set) = stream_shard(addr).read() {
        if set.contains(&addr) {
            return Some(stream as *mut NetStream);
        }
    }
    None
}

enum NetStream {
    Closed,
    Tcp(TcpStream, usize),
    Tls(TlsStream<TcpStream>, usize),
}

struct NetListener {
    listener: TcpListener,
    token: usize,
}

fn string_from_bytes(bytes: &[u8]) -> i64 {
    unsafe { new_string_from_bytes(bytes.as_ptr(), bytes.len() as i64) }
}

fn empty_string() -> i64 {
    unsafe { rt_string_from_c_str("\0".as_ptr() as *const _) }
}

fn lookup_all(host: &str) -> Vec<String> {
    let mut results: Vec<String> = Vec::new();
    if let Ok(addrs) = (host, 0u16).to_socket_addrs() {
        for addr in addrs {
            let ip = addr.ip().to_string();
            if !results.iter().any(|entry| entry == &ip) {
                results.push(ip);
            }
        }
    }
    results
}

fn connect_host_port(host: &str, port: i64) -> Option<(TcpStream, usize)> {
    let port_num = if port <= 0 || port > 65535 {
        return None;
    } else {
        port as u16
    };
    let addrs = (host, port_num).to_socket_addrs().ok()?;
    for addr in addrs {
        // TcpStream::connect in mio is non-blocking. It returns Ok immediately.
        if let Ok(mut stream) = TcpStream::connect(addr) {
            let _ = stream.set_nodelay(true);
            #[cfg(target_os = "macos")]
            {
                use std::os::unix::io::AsRawFd;
                let fd = stream.as_raw_fd();
                let one: libc::c_int = 1;
                unsafe {
                    libc::setsockopt(
                        fd,
                        libc::SOL_SOCKET,
                        libc::SO_NOSIGPIPE,
                        &one as *const _ as *const libc::c_void,
                        std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                    );
                }
            }
            let token = crate::vthread::vt_register_io(&mut stream);
            // wait for the connection to establish
            crate::vthread::vt_wait_io(token);
            if let Ok(Some(_err)) = stream.take_error() {
                crate::vthread::vt_deregister_io(&mut stream, token);
                continue;
            }
            return Some((stream, token));
        }
    }
    None
}

fn connect_tls_host(host: &str, port: i64, verify: bool) -> Option<(TlsStream<TcpStream>, usize)> {
    let (stream, token) = connect_host_port(host, port)?;
    let mut builder = TlsConnector::builder();
    if !verify {
        builder.danger_accept_invalid_certs(true);
        builder.danger_accept_invalid_hostnames(true);
    }
    let connector = builder.build().ok()?;
    
    let mut res = connector.connect(host, stream);
    loop {
        match res {
            Ok(tls) => return Some((tls, token)),
            Err(native_tls::HandshakeError::WouldBlock(mid)) => {
                crate::vthread::vt_wait_io(token);
                res = mid.handshake();
            }
            Err(_) => return None,
        }
    }
}

fn blocking_http_fetch(
    host: String,
    port: i64,
    use_tls: bool,
    request: String,
    _timeout_ms: i64,
    insecure_tls: bool,
) -> Result<Vec<u8>, String> {
    let mut stream = if use_tls {
        connect_tls_host(&host, port, !insecure_tls)
            .map(|(s, t)| NetStream::Tls(s, t))
            .ok_or_else(|| format!("Failed to connect to {}:{}", host, port))?
    } else {
        connect_host_port(&host, port)
            .map(|(s, t)| NetStream::Tcp(s, t))
            .ok_or_else(|| format!("Failed to connect to {}:{}", host, port))?
    };
    // Note: timeouts are not fully integrated with mio edge-trigger wait yet.
    // For now, they will just block indefinitely or we rely on the kernel's TCP timeout.
    stream_write_all(&mut stream, request.as_bytes())
        .map_err(|_| format!("Failed to write request to {}", host))?;
    let bytes = read_all(&mut stream, 4096);
    if bytes.is_empty() {
        return Err(format!("Empty response from {}", host));
    }
    Ok(bytes)
}

unsafe fn http_result_array(status: &str, payload: &[u8]) -> i64 {
    let mut result = rt_Array_new_fixed(2, 8);
    rt_push_root(&mut result);
    let mut status_id = string_from_bytes(status.as_bytes());
    rt_push_root(&mut status_id);
    let mut payload_id = string_from_bytes(payload);
    rt_push_root(&mut payload_id);
    rt_array_set_fast(result, 0, status_id);
    rt_array_set_fast(result, 1, payload_id);
    rt_pop_roots(3);
    result
}

fn start_tls_stream(stream: &mut NetStream, host: &str, verify: bool) -> bool {
    let current = std::mem::replace(stream, NetStream::Closed);
    match current {
        NetStream::Closed => false,
        NetStream::Tls(socket, token) => { *stream = NetStream::Tls(socket, token); true }
        NetStream::Tcp(socket, token) => {
            let mut builder = TlsConnector::builder();
            if !verify {
                builder.danger_accept_invalid_certs(true);
                builder.danger_accept_invalid_hostnames(true);
            }
            let Ok(connector) = builder.build() else { return false; };
            let mut res = connector.connect(host, socket);
            loop {
                match res {
                    Ok(tls) => { *stream = NetStream::Tls(tls, token); return true; }
                    Err(native_tls::HandshakeError::WouldBlock(mid)) => {
                        crate::vthread::vt_wait_io(token);
                        res = mid.handshake();
                    }
                    Err(_) => return false,
                }
            }
        }
    }
}

fn set_stream_timeout(_stream: &mut NetStream, _ms: i64) -> std::io::Result<()> {
    // With mio, timeouts should be handled by the netpoller.
    // For now, we stub this out as it requires a timer wheel in the scheduler.
    Ok(())
}

fn stream_write_all(stream: &mut NetStream, data: &[u8]) -> std::io::Result<usize> {
    let mut written = 0;
    while written < data.len() {
        let (res, token) = match stream {
            NetStream::Closed => return Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "closed")),
            NetStream::Tcp(s, token) => (s.write(&data[written..]), *token),
            NetStream::Tls(s, token) => (s.write(&data[written..]), *token),
        };
        match res {
            Ok(0) => return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "failed to write whole buffer")),
            Ok(n) => written += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                crate::vthread::vt_wait_io(token);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(written)
}

fn stream_read_once(stream: &mut NetStream, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        let (res, token) = match stream {
            NetStream::Closed => return Err(std::io::Error::new(std::io::ErrorKind::NotConnected, "closed")),
            NetStream::Tcp(s, token) => (s.read(buf), *token),
            NetStream::Tls(s, token) => (s.read(buf), *token),
        };
        match res {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                crate::vthread::vt_wait_io(token);
            }
            other => return other,
        }
    }
}

fn read_all(stream: &mut NetStream, chunk_size: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let size = if chunk_size == 0 { 4096 } else { chunk_size };
    let mut buf = vec![0u8; size.min(65536)];
    loop {
        match stream_read_once(stream, &mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => break,
        }
    }
    out
}

fn read_exact(stream: &mut NetStream, expected_len: usize) -> Vec<u8> {
    let mut out = vec![0u8; expected_len];
    let mut offset = 0;
    while offset < expected_len {
        match stream_read_once(stream, &mut out[offset..]) {
            Ok(0) => { out.truncate(offset); break; }
            Ok(n) => { offset += n; }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => { out.truncate(offset); break; }
            Err(_) => { out.truncate(offset); break; }
        }
    }
    out
}

unsafe fn new_string_array(items: Vec<String>) -> i64 {
    let mut result = rt_Array_new_fixed(0, 8);
    rt_push_root(&mut result);
    for item in items {
        let mut item_id = string_from_bytes(item.as_bytes());
        rt_push_root(&mut item_id);
        result = rt_array_push(result, item_id);
        rt_pop_roots(1);
    }
    rt_pop_roots(1);
    result
}

// ── Public C API ─────────────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn rt_net_lookup(host_ptr: i64) -> i64 {
    if let Some(host) = i64_to_rust_str(host_ptr) {
        crate::vthread::vt_with_syscall(|| {
            let addrs = lookup_all(&host);
            if let Some(first) = addrs.first() {
                return string_from_bytes(first.as_bytes());
            }
            0
        })
    } else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_lookup_all(host_ptr: i64) -> i64 {
    if let Some(host) = i64_to_rust_str(host_ptr) {
        return crate::vthread::vt_with_syscall(|| new_string_array(lookup_all(&host)));
    }
    rt_Array_new_fixed(0, 8)
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_connect(addr_ptr: i64) -> i64 {
    if let Some(addr_str) = i64_to_rust_str(addr_ptr) {
        if let Some(addr) = addr_str.as_str().to_socket_addrs().ok().and_then(|mut iter| iter.next()) {
            if let Ok(mut stream) = TcpStream::connect(addr) {
                let _ = stream.set_nodelay(true);
                let token = crate::vthread::vt_register_io(&mut stream);
                crate::vthread::vt_wait_io(token);
                if let Ok(Some(_)) = stream.take_error() {
                    crate::vthread::vt_deregister_io(&mut stream, token);
                    return -1;
                }
                return register_stream_ptr(Box::into_raw(Box::new(NetStream::Tcp(stream, token))));
            }
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_connect_host(host_ptr: i64, port: i64) -> i64 {
    if let Some(host) = i64_to_rust_str(host_ptr) {
        if let Some((s, token)) = connect_host_port(&host, port) {
            return register_stream_ptr(Box::into_raw(Box::new(NetStream::Tcp(s, token))));
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_connect_tls(host_ptr: i64, port: i64) -> i64 {
    if let Some(host) = i64_to_rust_str(host_ptr) {
        if let Some((s, token)) = connect_tls_host(&host, port, true) {
            return register_stream_ptr(Box::into_raw(Box::new(NetStream::Tls(s, token))));
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_connect_tls_insecure(host_ptr: i64, port: i64) -> i64 {
    if let Some(host) = i64_to_rust_str(host_ptr) {
        if let Some((s, token)) = connect_tls_host(&host, port, false) {
            return register_stream_ptr(Box::into_raw(Box::new(NetStream::Tls(s, token))));
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_send(stream: i64, data: i64) -> i64 {
    let Some(ptr) = validate_stream_ptr(stream) else { return -1; };
    if let Some((bytes, len)) = get_str_parts(data) {
        let payload = std::slice::from_raw_parts(bytes, len as usize);
        let socket = &mut *ptr;
        return match stream_write_all(socket, payload) {
            Ok(n) => n as i64,
            Err(_) => -1,
        };
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_send_bytes(stream: i64, data: i64) -> i64 {
    let Some(ptr) = validate_stream_ptr(stream) else { return -1; };
    let Some(bytes) = bytes_from_int_array(data) else { return -1; };
    let socket = &mut *ptr;
    match stream_write_all(socket, &bytes) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_receive(stream: i64, max_len: i64) -> i64 {
    let Some(ptr) = validate_stream_ptr(stream) else { return empty_string(); };
    let size = if max_len <= 0 { 4096 } else { max_len as usize };
    let socket = &mut *ptr;
    let mut buf = vec![0u8; size];
    match stream_read_once(socket, &mut buf) {
        Ok(n) => string_from_bytes(&buf[..n]),
        Err(_) => empty_string(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_receive_bytes(stream: i64, max_len: i64) -> i64 {
    let Some(ptr) = validate_stream_ptr(stream) else { return rt_Array_new(0, 4); };
    let size = if max_len <= 0 { 4096 } else { max_len as usize };
    let socket = &mut *ptr;
    let mut buf = vec![0u8; size];
    match stream_read_once(socket, &mut buf) {
        Ok(n) => int_array_from_bytes(&buf[..n]),
        Err(_) => rt_Array_new(0, 4),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_read_all(stream: i64, chunk_size: i64) -> i64 {
    let Some(ptr) = validate_stream_ptr(stream) else { return empty_string(); };
    let size = if chunk_size <= 0 { 4096 } else { chunk_size as usize };
    let socket = &mut *ptr;
    string_from_bytes(&read_all(socket, size))
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_read_all_bytes(stream: i64, chunk_size: i64) -> i64 {
    let Some(ptr) = validate_stream_ptr(stream) else { return rt_Array_new(0, 4); };
    let size = if chunk_size <= 0 { 4096 } else { chunk_size as usize };
    let socket = &mut *ptr;
    int_array_from_bytes(&read_all(socket, size))
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_read_exact_bytes(stream: i64, expected_len: i64) -> i64 {
    let Some(ptr) = validate_stream_ptr(stream) else { return rt_Array_new(0, 4); };
    if expected_len <= 0 { return rt_Array_new(0, 4); }
    let socket = &mut *ptr;
    int_array_from_bytes(&read_exact(socket, expected_len as usize))
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_set_timeout(stream: i64, timeout_ms: i64) -> i64 {
    let Some(ptr) = validate_stream_ptr(stream) else { return -1; };
    let socket = &mut *ptr;
    match set_stream_timeout(socket, timeout_ms) { Ok(_) => 0, Err(_) => -1 }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_start_tls(stream: i64, host_ptr: i64) -> i64 {
    let Some(ptr) = validate_stream_ptr(stream) else { return -1; };
    let Some(host) = i64_to_rust_str(host_ptr) else { return -1; };
    let socket = &mut *ptr;
    if start_tls_stream(socket, &host, true) { 0 } else { -1 }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_start_tls_insecure(stream: i64, host_ptr: i64) -> i64 {
    let Some(ptr) = validate_stream_ptr(stream) else { return -1; };
    let Some(host) = i64_to_rust_str(host_ptr) else { return -1; };
    let socket = &mut *ptr;
    if start_tls_stream(socket, &host, false) { 0 } else { -1 }
}

#[no_mangle]
pub unsafe extern "C" fn rt_http_fetch(
    host_ptr: i64, port: i64, use_tls: i64,
    request_ptr: i64, timeout_ms: i64, insecure_tls: i64,
) -> i64 {
    let Some(host)    = i64_to_rust_str(host_ptr)    else { return http_result_array("err", b"Invalid host"); };
    let Some(request) = i64_to_rust_str(request_ptr) else { return http_result_array("err", b"Invalid request"); };
    match blocking_http_fetch(host, port, use_tls != 0, request, timeout_ms, insecure_tls != 0) {
        Ok(bytes) => http_result_array("ok", &bytes),
        Err(err)  => http_result_array("err", err.as_bytes()),
    }
}

thread_local! {
    static LAST_NET_ERROR: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
}

pub fn set_last_net_error(err: &str) {
    LAST_NET_ERROR.with(|cell| *cell.borrow_mut() = err.to_string());
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_last_error() -> i64 {
    let s = LAST_NET_ERROR.with(|cell| cell.borrow().clone());
    rt_string_from_owned_string(s)
}

// ── Listen / Accept ───────────────────────────────────────────────────────────

#[no_mangle]
pub unsafe extern "C" fn rt_net_listen(addr_ptr: i64) -> i64 {
    #[cfg(unix)]
    {
        let mut rl = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) == 0 {
            let target = rl.rlim_max.min(65535);
            if rl.rlim_cur < target {
                rl.rlim_cur = target;
                libc::setrlimit(libc::RLIMIT_NOFILE, &rl);
            }
        }
    }

    let Some(addr_str) = i64_to_rust_str(addr_ptr) else {
        set_last_net_error("Invalid address argument");
        return -1;
    };
    let Some(addr) = addr_str.as_str().to_socket_addrs().ok().and_then(|mut iter| iter.next()) else {
        set_last_net_error(&format!("Invalid socket address: '{}'", addr_str));
        return -1;
    };
    
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        let domain = if addr.is_ipv4() { libc::AF_INET } else { libc::AF_INET6 };
        let fd = libc::socket(domain, libc::SOCK_STREAM, 0);
        if fd < 0 {
            let err = std::io::Error::last_os_error();
            set_last_net_error(&format!("Socket creation failed: {}", err));
            return -1;
        }

        let one: libc::c_int = 1;
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEADDR, &one as *const _ as *const libc::c_void, std::mem::size_of::<libc::c_int>() as libc::socklen_t);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEPORT, &one as *const _ as *const libc::c_void, std::mem::size_of::<libc::c_int>() as libc::socklen_t);

        let res = match addr {
            std::net::SocketAddr::V4(v4) => {
                let mut sin = std::mem::zeroed::<libc::sockaddr_in>();
                sin.sin_family = libc::AF_INET as libc::sa_family_t;
                sin.sin_port = v4.port().to_be();
                sin.sin_addr.s_addr = u32::from_ne_bytes(v4.ip().octets());
                libc::bind(fd, &sin as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t)
            }
            std::net::SocketAddr::V6(v6) => {
                let mut sin6 = std::mem::zeroed::<libc::sockaddr_in6>();
                sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                sin6.sin6_port = v6.port().to_be();
                sin6.sin6_addr.s6_addr = v6.ip().octets();
                libc::bind(fd, &sin6 as *const _ as *const libc::sockaddr, std::mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t)
            }
        };

        if res < 0 {
            let err = std::io::Error::last_os_error();
            let desc = if err.raw_os_error() == Some(libc::EADDRINUSE) {
                format!("Address already in use: Port {} is already in use by another service (EADDRINUSE)", addr.port())
            } else if err.raw_os_error() == Some(libc::EACCES) {
                format!("Permission denied: Cannot bind to port {} (EACCES)", addr.port())
            } else {
                format!("Bind failed on {}: {}", addr, err)
            };
            set_last_net_error(&desc);
            libc::close(fd);
            return -1;
        }

        if libc::listen(fd, 65535) < 0 {
            let err = std::io::Error::last_os_error();
            set_last_net_error(&format!("Listen failed on {}: {}", addr, err));
            libc::close(fd);
            return -1;
        }

        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);

        let std_listener = std::net::TcpListener::from_raw_fd(fd);
        let mut listener = TcpListener::from_std(std_listener);
        let token = crate::vthread::vt_register_io(&mut listener);
        let net_listener = NetListener { listener, token };
        return register_listener_ptr(Box::into_raw(Box::new(net_listener)));
    }

    #[cfg(not(unix))]
    {
        match TcpListener::bind(addr) {
            Err(_) => -1,
            Ok(mut listener) => {
                let token = crate::vthread::vt_register_io(&mut listener);
                let net_listener = NetListener { listener, token };
                register_listener_ptr(Box::into_raw(Box::new(net_listener)))
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_accept(listener_ptr: i64) -> i64 {
    if listener_ptr <= 0 { return -1; }
    let net_listener = &*(listener_ptr as *const NetListener);
    loop {
        match net_listener.listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nodelay(true);
                #[cfg(target_os = "macos")]
                {
                    use std::os::unix::io::AsRawFd;
                    let fd = stream.as_raw_fd();
                    let one: libc::c_int = 1;
                    libc::setsockopt(
                        fd,
                        libc::SOL_SOCKET,
                        libc::SO_NOSIGPIPE,
                        &one as *const _ as *const libc::c_void,
                        std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                    );
                }
                let token = crate::vthread::vt_register_io(&mut stream);
                return register_stream_ptr(Box::into_raw(Box::new(NetStream::Tcp(stream, token))));
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted
                   || e.kind() == std::io::ErrorKind::WouldBlock => {
                crate::vthread::vt_wait_io(net_listener.token);
            }
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionAborted
                   || e.kind() == std::io::ErrorKind::ConnectionReset => {
                continue;
            }
            Err(e) if e.raw_os_error() == Some(libc::EMFILE)
                   || e.raw_os_error() == Some(libc::ENFILE) => {
                crate::vthread::vt_sleep(1);
            }
            Err(_) => {
                crate::vthread::vt_sleep(1);
                continue;
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_close_listener(listener_ptr: i64) -> i64 {
    if listener_ptr <= 0 { return -1; }
    let addr = listener_ptr as usize;
    let removed = if let Ok(mut set) = LIVE_LISTENERS.lock() {
        set.remove(&addr)
    } else {
        false
    };
    if !removed { return 0; }
    let mut net_listener = Box::from_raw(listener_ptr as *mut NetListener);
    crate::vthread::vt_deregister_io(&mut net_listener.listener, net_listener.token);
    0
}

fn close_net_stream(stream: NetStream) {
    match stream {
        NetStream::Closed => {}
        NetStream::Tcp(mut tcp, token) => {
            crate::vthread::vt_deregister_io(&mut tcp, token);
        }
        NetStream::Tls(mut tls, token) => {
            crate::vthread::vt_deregister_io(tls.get_mut(), token);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_close(stream: i64) -> i64 {
    if stream <= 0 { return -1; }
    let addr = stream as usize;
    let removed = if let Ok(mut set) = stream_shard(addr).write() {
        set.remove(&addr)
    } else {
        false
    };
    if !removed { return 0; }
    let ptr = stream as *mut NetStream;
    let old = std::mem::replace(&mut *ptr, NetStream::Closed);
    close_net_stream(old);
    drop(Box::from_raw(ptr));
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_close_stream_obj(this: i64) -> i64 {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return -1; }
    let atomic_slot = ptr.offset(0) as *const AtomicI64;
    let id = (*atomic_slot).swap(0, Ordering::AcqRel);
    if id <= 0 { return 0; }
    rt_net_close(id)
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_close_listener_obj(this: i64) -> i64 {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { return -1; }
    let atomic_slot = ptr.offset(0) as *const AtomicI64;
    let id = (*atomic_slot).swap(0, Ordering::AcqRel);
    if id <= 0 { return 0; }
    rt_net_close_listener(id)
}

#[no_mangle]
pub unsafe extern "C" fn rt_TcpStream_constructor(this: i64, id: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { if id > 0 { let _ = rt_net_close(id); } return; }
    rt_ensure_type_finalizer(this, rt_tcp_stream_object_finalizer);
    *ptr.offset(0) = id;
}

#[no_mangle]
pub unsafe extern "C" fn rt_TcpListener_constructor(this: i64, id: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() { if id > 0 { let _ = rt_net_close_listener(id); } return; }
    rt_ensure_type_finalizer(this, rt_tcp_listener_object_finalizer);
    *ptr.offset(0) = id;
}
