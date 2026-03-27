use super::*;
use native_tls::{TlsConnector, TlsStream};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::time::Duration;

enum NetStream {
    Closed,
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
}

enum HttpFetchResultData {
    Ok(Vec<u8>),
    Err(String),
}

static HTTP_FETCH_RESULTS: LazyLock<Mutex<std::collections::HashMap<usize, HttpFetchResultData>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

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

fn connect_host_port(host: &str, port: i64) -> Option<TcpStream> {
    let port_num = if port <= 0 || port > 65535 {
        return None;
    } else {
        port as u16
    };

    let addrs = (host, port_num).to_socket_addrs().ok()?;
    for addr in addrs {
        if let Ok(stream) = TcpStream::connect(addr) {
            let _ = stream.set_nodelay(true);
            return Some(stream);
        }
    }
    None
}

fn connect_tls_host(host: &str, port: i64, verify: bool) -> Option<TlsStream<TcpStream>> {
    let stream = connect_host_port(host, port)?;
    let mut builder = TlsConnector::builder();
    if !verify {
        builder.danger_accept_invalid_certs(true);
        builder.danger_accept_invalid_hostnames(true);
    }
    let connector = builder.build().ok()?;
    connector.connect(host, stream).ok()
}

fn blocking_http_fetch(
    host: String,
    port: i64,
    use_tls: bool,
    request: String,
    timeout_ms: i64,
    insecure_tls: bool,
) -> Result<Vec<u8>, String> {
    let mut stream = if use_tls {
        connect_tls_host(&host, port, !insecure_tls)
            .map(NetStream::Tls)
            .ok_or_else(|| format!("Failed to connect to {}:{}", host, port))?
    } else {
        connect_host_port(&host, port)
            .map(NetStream::Tcp)
            .ok_or_else(|| format!("Failed to connect to {}:{}", host, port))?
    };

    let _ = set_stream_timeout(&mut stream, timeout_ms);
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

#[no_mangle]
pub unsafe extern "C" fn rt_http_fetch_resolver_worker(handle_id: i64) {
    let handle = handle_id as usize;
    let actual_pid = crate::event_loop::tejx_get_global_handle(handle);
    let fetch_result = HTTP_FETCH_RESULTS
        .lock()
        .ok()
        .and_then(|mut results| results.remove(&handle));

    crate::event_loop::tejx_drop_global_handle(handle);

    if actual_pid > 0 {
        let value = match fetch_result {
            Some(HttpFetchResultData::Ok(bytes)) => http_result_array("ok", &bytes),
            Some(HttpFetchResultData::Err(err)) => http_result_array("err", err.as_bytes()),
            None => http_result_array("err", b"Fetch result unavailable"),
        };
        rt_promise_resolve(actual_pid, value);
    }

    crate::event_loop::tejx_dec_async_ops();
}

fn start_tls_stream(stream: &mut NetStream, host: &str, verify: bool) -> bool {
    let current = std::mem::replace(stream, NetStream::Closed);
    match current {
        NetStream::Closed => false,
        NetStream::Tls(socket) => {
            *stream = NetStream::Tls(socket);
            true
        }
        NetStream::Tcp(socket) => {
            let mut builder = TlsConnector::builder();
            if !verify {
                builder.danger_accept_invalid_certs(true);
                builder.danger_accept_invalid_hostnames(true);
            }
            let Ok(connector) = builder.build() else {
                return false;
            };

            match connector.connect(host, socket) {
                Ok(tls) => {
                    *stream = NetStream::Tls(tls);
                    true
                }
                Err(_) => false,
            }
        }
    }
}

fn set_stream_timeout(stream: &mut NetStream, ms: i64) -> std::io::Result<()> {
    let timeout = if ms < 0 {
        None
    } else {
        Some(Duration::from_millis(ms as u64))
    };

    match stream {
        NetStream::Closed => {}
        NetStream::Tcp(socket) => {
            socket.set_read_timeout(timeout)?;
            socket.set_write_timeout(timeout)?;
        }
        NetStream::Tls(socket) => {
            let inner = socket.get_mut();
            inner.set_read_timeout(timeout)?;
            inner.set_write_timeout(timeout)?;
        }
    }

    Ok(())
}

fn stream_write_all(stream: &mut NetStream, data: &[u8]) -> std::io::Result<usize> {
    match stream {
        NetStream::Closed => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "stream is closed",
            ));
        }
        NetStream::Tcp(socket) => {
            socket.write_all(data)?;
        }
        NetStream::Tls(socket) => {
            socket.write_all(data)?;
        }
    }
    Ok(data.len())
}

fn stream_read_once(stream: &mut NetStream, buf: &mut [u8]) -> std::io::Result<usize> {
    match stream {
        NetStream::Closed => Err(std::io::Error::new(
            std::io::ErrorKind::NotConnected,
            "stream is closed",
        )),
        NetStream::Tcp(socket) => socket.read(buf),
        NetStream::Tls(socket) => socket.read(buf),
    }
}

fn read_all(stream: &mut NetStream, chunk_size: usize) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let size = if chunk_size == 0 { 4096 } else { chunk_size };
    let mut buf = vec![0u8; size];

    loop {
        match stream_read_once(stream, &mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                break;
            }
            Err(_) => break,
        }
    }

    out
}

fn read_exact(stream: &mut NetStream, expected_len: usize) -> Vec<u8> {
    let mut out = vec![0u8; expected_len];
    let mut offset = 0usize;

    while offset < expected_len {
        match stream_read_once(stream, &mut out[offset..]) {
            Ok(0) => {
                out.truncate(offset);
                break;
            }
            Ok(n) => {
                offset += n;
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                out.truncate(offset);
                break;
            }
            Err(_) => {
                out.truncate(offset);
                break;
            }
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

#[no_mangle]
pub unsafe extern "C" fn rt_net_lookup(host_ptr: i64) -> i64 {
    if let Some(host) = i64_to_rust_str(host_ptr) {
        let addrs = lookup_all(&host);
        if let Some(first) = addrs.first() {
            return string_from_bytes(first.as_bytes());
        }
    }
    empty_string()
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_lookup_all(host_ptr: i64) -> i64 {
    if let Some(host) = i64_to_rust_str(host_ptr) {
        return new_string_array(lookup_all(&host));
    }
    rt_Array_new_fixed(0, 8)
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_connect(addr_ptr: i64) -> i64 {
    if let Some(addr) = i64_to_rust_str(addr_ptr) {
        match TcpStream::connect(&addr) {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                let boxed = Box::new(NetStream::Tcp(stream));
                Box::into_raw(boxed) as i64
            }
            Err(_) => -1,
        }
    } else {
        -1
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_connect_host(host_ptr: i64, port: i64) -> i64 {
    if let Some(host) = i64_to_rust_str(host_ptr) {
        if let Some(stream) = connect_host_port(&host, port) {
            let boxed = Box::new(NetStream::Tcp(stream));
            return Box::into_raw(boxed) as i64;
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_connect_tls(host_ptr: i64, port: i64) -> i64 {
    if let Some(host) = i64_to_rust_str(host_ptr) {
        if let Some(stream) = connect_tls_host(&host, port, true) {
            let boxed = Box::new(NetStream::Tls(stream));
            return Box::into_raw(boxed) as i64;
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_connect_tls_insecure(host_ptr: i64, port: i64) -> i64 {
    if let Some(host) = i64_to_rust_str(host_ptr) {
        if let Some(stream) = connect_tls_host(&host, port, false) {
            let boxed = Box::new(NetStream::Tls(stream));
            return Box::into_raw(boxed) as i64;
        }
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_send(stream: i64, data: i64) -> i64 {
    if stream <= 0 {
        return -1;
    }

    if let Some((bytes, len)) = get_str_parts(data) {
        let slice = std::slice::from_raw_parts(bytes, len as usize);
        let socket = &mut *(stream as *mut NetStream);
        return match stream_write_all(socket, slice) {
            Ok(size) => size as i64,
            Err(_) => -1,
        };
    }

    -1
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_send_bytes(stream: i64, data: i64) -> i64 {
    if stream <= 0 {
        return -1;
    }

    let Some(bytes) = bytes_from_int_array(data) else {
        return -1;
    };

    let socket = &mut *(stream as *mut NetStream);
    match stream_write_all(socket, &bytes) {
        Ok(size) => size as i64,
        Err(_) => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_receive(stream: i64, max_len: i64) -> i64 {
    if stream <= 0 {
        return empty_string();
    }

    let size = if max_len <= 0 { 4096 } else { max_len as usize };
    let socket = &mut *(stream as *mut NetStream);
    let mut buf = vec![0u8; size];
    match stream_read_once(socket, &mut buf) {
        Ok(n) => string_from_bytes(&buf[..n]),
        Err(_) => empty_string(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_receive_bytes(stream: i64, max_len: i64) -> i64 {
    if stream <= 0 {
        return rt_Array_new(0, 4);
    }

    let size = if max_len <= 0 { 4096 } else { max_len as usize };
    let socket = &mut *(stream as *mut NetStream);
    let mut buf = vec![0u8; size];
    match stream_read_once(socket, &mut buf) {
        Ok(n) => int_array_from_bytes(&buf[..n]),
        Err(_) => rt_Array_new(0, 4),
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_read_all(stream: i64, chunk_size: i64) -> i64 {
    if stream <= 0 {
        return empty_string();
    }

    let size = if chunk_size <= 0 {
        4096
    } else {
        chunk_size as usize
    };
    let socket = &mut *(stream as *mut NetStream);
    let bytes = read_all(socket, size);
    string_from_bytes(&bytes)
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_read_all_bytes(stream: i64, chunk_size: i64) -> i64 {
    if stream <= 0 {
        return rt_Array_new(0, 4);
    }

    let size = if chunk_size <= 0 {
        4096
    } else {
        chunk_size as usize
    };
    let socket = &mut *(stream as *mut NetStream);
    let bytes = read_all(socket, size);
    int_array_from_bytes(&bytes)
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_read_exact_bytes(stream: i64, expected_len: i64) -> i64 {
    if stream <= 0 || expected_len <= 0 {
        return rt_Array_new(0, 4);
    }

    let socket = &mut *(stream as *mut NetStream);
    let bytes = read_exact(socket, expected_len as usize);
    int_array_from_bytes(&bytes)
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_set_timeout(stream: i64, timeout_ms: i64) -> i64 {
    if stream <= 0 {
        return -1;
    }

    let socket = &mut *(stream as *mut NetStream);
    match set_stream_timeout(socket, timeout_ms) {
        Ok(_) => 0,
        Err(_) => -1,
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_start_tls(stream: i64, host_ptr: i64) -> i64 {
    if stream <= 0 {
        return -1;
    }

    let Some(host) = i64_to_rust_str(host_ptr) else {
        return -1;
    };

    let socket = &mut *(stream as *mut NetStream);
    if start_tls_stream(socket, &host, true) {
        0
    } else {
        -1
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_start_tls_insecure(stream: i64, host_ptr: i64) -> i64 {
    if stream <= 0 {
        return -1;
    }

    let Some(host) = i64_to_rust_str(host_ptr) else {
        return -1;
    };

    let socket = &mut *(stream as *mut NetStream);
    if start_tls_stream(socket, &host, false) {
        0
    } else {
        -1
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_http_fetch_async(
    host_ptr: i64,
    port: i64,
    use_tls: i64,
    request_ptr: i64,
    timeout_ms: i64,
    insecure_tls: i64,
) -> i64 {
    let pid = rt_promise_new();
    let mut v_pid = pid;
    rt_push_root(&mut v_pid);

    let Some(host) = i64_to_rust_str(host_ptr) else {
        let mut result = http_result_array("err", b"Invalid fetch host");
        rt_push_root(&mut result);
        rt_promise_resolve(v_pid, result);
        rt_pop_roots(2);
        return pid;
    };

    let Some(request) = i64_to_rust_str(request_ptr) else {
        let mut result = http_result_array("err", b"Invalid fetch request");
        rt_push_root(&mut result);
        rt_promise_resolve(v_pid, result);
        rt_pop_roots(2);
        return pid;
    };

    let handle = crate::event_loop::tejx_create_global_handle(pid);
    crate::event_loop::tejx_inc_async_ops();
    rt_pop_roots(1);

    crate::event_loop::TOKIO_RT.spawn(async move {
        let fetch_result = match tokio::task::spawn_blocking(move || {
            blocking_http_fetch(host, port, use_tls != 0, request, timeout_ms, insecure_tls != 0)
        })
        .await
        {
            Ok(result) => result,
            Err(err) => Err(format!("Async fetch task failed: {}", err)),
        };

        if let Ok(mut results) = HTTP_FETCH_RESULTS.lock() {
            results.insert(
                handle,
                match fetch_result {
                    Ok(bytes) => HttpFetchResultData::Ok(bytes),
                    Err(err) => HttpFetchResultData::Err(err),
                },
            );
        }

        unsafe {
            crate::event_loop::tejx_enqueue_task(
                rt_http_fetch_resolver_worker as *const () as i64,
                handle as i64,
            );
        }
    });

    pid
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_listen(addr_ptr: i64) -> i64 {
    if let Some(addr) = i64_to_rust_str(addr_ptr) {
        match TcpListener::bind(&addr) {
            Ok(listener) => {
                let boxed = Box::new(listener);
                Box::into_raw(boxed) as i64
            }
            Err(_) => -1,
        }
    } else {
        -1
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_accept(listener_ptr: i64) -> i64 {
    if listener_ptr <= 0 {
        return -1;
    }

    let listener = &*(listener_ptr as *const TcpListener);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nodelay(true);
                let boxed = Box::new(NetStream::Tcp(stream));
                return Box::into_raw(boxed) as i64;
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {
                continue;
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                return 0;
            }
            Err(_) => {
                return -1;
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_close_listener(listener_ptr: i64) -> i64 {
    if listener_ptr <= 0 {
        return -1;
    }
    let _ = Box::from_raw(listener_ptr as *mut TcpListener);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_net_close(stream: i64) -> i64 {
    if stream <= 0 {
        return -1;
    }
    let _ = Box::from_raw(stream as *mut NetStream);
    0
}

#[no_mangle]
pub unsafe extern "C" fn rt_TcpStream_constructor(this: i64, id: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        if id > 0 {
            let _ = rt_net_close(id);
        }
        return;
    }
    rt_ensure_type_finalizer(this, rt_tcp_stream_object_finalizer);
    *ptr.offset(0) = id;
}

#[no_mangle]
pub unsafe extern "C" fn rt_TcpListener_constructor(this: i64, id: i64) {
    let ptr = rt_obj_ptr(this);
    if ptr.is_null() {
        if id > 0 {
            let _ = rt_net_close_listener(id);
        }
        return;
    }
    rt_ensure_type_finalizer(this, rt_tcp_listener_object_finalizer);
    *ptr.offset(0) = id;
}
