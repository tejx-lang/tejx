use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::ffi::CString;
use std::sync::{Arc, Mutex, Condvar};
use crate::runtime::{HEAP, TaggedValue, stringify_value, PromiseState};

pub fn exports() -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("connect".to_string());
    s.insert("send".to_string());
    s.insert("receive".to_string());
    s.insert("close".to_string());
    s
}

pub fn http_exports() -> HashSet<String> {
    let mut s = HashSet::new();
    let methods = ["get", "post", "put", "delete", "patch", "head", "options"];
    for m in methods {
        s.insert(m.to_string());
        s.insert(format!("{}Sync", m));
    }
    s
}

pub fn https_exports() -> HashSet<String> {
    http_exports()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_net_connect(addr_id: i64) -> i64 {
    let addr = stringify_value(addr_id);
    match TcpStream::connect(&addr) {
        Ok(stream) => {
            let mut heap = HEAP.lock().unwrap();
            let id = heap.next_id;
            heap.next_id += 1;
            heap.insert(id, TaggedValue::TCPStream(Arc::new(Mutex::new(stream))));
            id
        }
        Err(_) => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_net_send(stream_id: i64, data_id: i64) -> i64 {
    let data = stringify_value(data_id);
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::TCPStream(stream_mutex)) = heap.get(stream_id) {
        let mut stream = stream_mutex.lock().unwrap();
        match stream.write_all(data.as_bytes()) {
            Ok(_) => return 0,
            Err(_) => return -1,
        }
    }
    -1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_net_receive(stream_id: i64, max_len: i64) -> i64 {
    let heap = HEAP.lock().unwrap();
    if let Some(TaggedValue::TCPStream(stream_mutex)) = heap.get(stream_id) {
        let mut stream = stream_mutex.lock().unwrap();
        let mut buf = vec![0u8; max_len as usize];
        match stream.read(&mut buf) {
            Ok(n) => {
                let s = String::from_utf8_lossy(&buf[..n]).to_string();
                let c_str = CString::new(s).unwrap();
                return c_str.into_raw() as i64;
            }
            Err(_) => return 0,
        }
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn std_net_close(stream_id: i64) -> i64 {
    let mut heap = HEAP.lock().unwrap();
    let idx = stream_id as usize;
    if idx < heap.objects.len() {
        heap.objects[idx] = None;
        return 0;
    }
    -1
}

// Sync API Implementation
fn std_net_http_request_sync(method: &str, url: &str, body: Option<&str>) -> i64 {
    use std::process::Command;
    let mut cmd = Command::new("/usr/bin/curl");
    cmd.arg("-s").arg("-X").arg(method).arg(url);
    if let Some(b) = body {
        cmd.arg("-d").arg(b);
    }
    match cmd.output() {
        Ok(output) => {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout).to_string();
                let mut heap = HEAP.lock().unwrap();
                return heap.alloc(TaggedValue::String(s));
            } else {
                 let _ = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/tejx_debug.log").map(|mut f| {
                    use std::io::Write;
                    let _ = writeln!(f, "Sync Request Failed ({} {}). Status: {:?}. Stderr: {}", method, url, output.status, String::from_utf8_lossy(&output.stderr));
                });
            }
            0
        }
        Err(e) => {
             let _ = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/tejx_debug.log").map(|mut f| {
                use std::io::Write;
                let _ = writeln!(f, "Sync Request execution failed: {}", e);
            });
            0
        }
    }
}

// Async API Helper
fn std_net_http_request_async(method: String, url: String, body: Option<String>) -> i64 {
    use std::thread;

    let promise: Arc<(Mutex<PromiseState>, Condvar)> = Arc::new((Mutex::new(PromiseState::Pending), Condvar::new()));
    let p_clone = Arc::clone(&promise);
    
    let promise_id = {
        let mut heap = HEAP.lock().unwrap();
        heap.alloc(TaggedValue::Promise(promise))
    };
    
    thread::spawn(move || {
        let result = std_net_http_request_internal(&method, &url, body.as_deref());
        let lock: &Mutex<PromiseState> = &p_clone.0;
        let cvar: &Condvar = &p_clone.1;
        let mut state = lock.lock().unwrap();
        if result != 0 {
            *state = PromiseState::Resolved(result);
        } else {
            *state = PromiseState::Rejected(0);
        }
        cvar.notify_all();
    });
    
    promise_id
}

fn std_net_http_request_internal(method: &str, url: &str, body: Option<&str>) -> i64 {
    use std::process::Command;
    let mut cmd = Command::new("/usr/bin/curl");
    cmd.arg("-s").arg("-X").arg(method).arg(url);
    if let Some(b) = body {
        cmd.arg("-d").arg(b);
    }
    match cmd.output() {
        Ok(output) => {
            if output.status.success() {
                let s = String::from_utf8_lossy(&output.stdout).to_string();
                let mut heap = HEAP.lock().unwrap();
                let id = heap.alloc(TaggedValue::String(s));
                let _ = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/tejx_debug.log").map(|mut f| {
                    use std::io::Write;
                    let _ = writeln!(f, "Request successful. Allocated ID: {}", id);
                });
                return id;
            } else {
                let _ = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/tejx_debug.log").map(|mut f| {
                    use std::io::Write;
                    let _ = writeln!(f, "Curl failed with status: {:?}. Stderr: {}", output.status, String::from_utf8_lossy(&output.stderr));
                });
            }
            0
        }
        Err(e) => {
            let _ = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/tejx_debug.log").map(|mut f| {
                use std::io::Write;
                let _ = writeln!(f, "Failed to execute /usr/bin/curl: {}", e);
            });
            0
        }
    }
}

// Exported Sync Methods
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_getSync(url_id: i64) -> i64 { std_net_http_request_sync("GET", &stringify_value(url_id), None) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_postSync(url_id: i64, body_id: i64) -> i64 { std_net_http_request_sync("POST", &stringify_value(url_id), Some(&stringify_value(body_id))) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_putSync(url_id: i64, body_id: i64) -> i64 { std_net_http_request_sync("PUT", &stringify_value(url_id), Some(&stringify_value(body_id))) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_deleteSync(url_id: i64) -> i64 { std_net_http_request_sync("DELETE", &stringify_value(url_id), None) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_patchSync(url_id: i64, body_id: i64) -> i64 { std_net_http_request_sync("PATCH", &stringify_value(url_id), Some(&stringify_value(body_id))) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_headSync(url_id: i64) -> i64 { std_net_http_request_sync("HEAD", &stringify_value(url_id), None) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_optionsSync(url_id: i64) -> i64 { std_net_http_request_sync("OPTIONS", &stringify_value(url_id), None) }

// Exported Async Methods
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_get(url_id: i64) -> i64 { std_net_http_request_async("GET".to_string(), stringify_value(url_id), None) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_post(url_id: i64, body_id: i64) -> i64 { std_net_http_request_async("POST".to_string(), stringify_value(url_id), Some(stringify_value(body_id))) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_put(url_id: i64, body_id: i64) -> i64 { std_net_http_request_async("PUT".to_string(), stringify_value(url_id), Some(stringify_value(body_id))) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_delete(url_id: i64) -> i64 { std_net_http_request_async("DELETE".to_string(), stringify_value(url_id), None) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_patch(url_id: i64, body_id: i64) -> i64 { std_net_http_request_async("PATCH".to_string(), stringify_value(url_id), Some(stringify_value(body_id))) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_head(url_id: i64) -> i64 { std_net_http_request_async("HEAD".to_string(), stringify_value(url_id), None) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_http_options(url_id: i64) -> i64 { std_net_http_request_async("OPTIONS".to_string(), stringify_value(url_id), None) }

// HTTPS versions
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_getSync(url_id: i64) -> i64 { std_http_getSync(url_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_postSync(url_id: i64, body_id: i64) -> i64 { std_http_postSync(url_id, body_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_putSync(url_id: i64, body_id: i64) -> i64 { std_http_putSync(url_id, body_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_deleteSync(url_id: i64) -> i64 { std_http_deleteSync(url_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_patchSync(url_id: i64, body_id: i64) -> i64 { std_http_patchSync(url_id, body_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_headSync(url_id: i64) -> i64 { std_http_headSync(url_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_optionsSync(url_id: i64) -> i64 { std_http_optionsSync(url_id) }

#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_get(url_id: i64) -> i64 { std_http_get(url_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_post(url_id: i64, body_id: i64) -> i64 { std_http_post(url_id, body_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_put(url_id: i64, body_id: i64) -> i64 { std_http_put(url_id, body_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_delete(url_id: i64) -> i64 { std_http_delete(url_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_patch(url_id: i64, body_id: i64) -> i64 { std_http_patch(url_id, body_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_head(url_id: i64) -> i64 { std_http_head(url_id) }
#[unsafe(no_mangle)] pub unsafe extern "C" fn std_https_options(url_id: i64) -> i64 { std_http_options(url_id) }
