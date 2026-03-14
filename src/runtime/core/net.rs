use super::*; // Extracted \n
#[no_mangle]
pub unsafe extern "C" fn rt_net_connect(addr: i64) -> i64 {
    if let Some(address) = i64_to_rust_str(addr) {
        match std::net::TcpStream::connect(&address) {
            Ok(stream) => {
                let boxed = Box::new(stream);
                Box::into_raw(boxed) as i64
            }
            Err(_) => -1,
        }
    } else {
        -1
    }
}
#[no_mangle]
pub unsafe extern "C" fn rt_net_send(stream: i64, data: i64) -> i64 {
    if stream <= 0 {
        return -1;
    }
    if let Some((bytes, len)) = get_str_parts(data) {
        let slice = std::slice::from_raw_parts(bytes, len as usize);
        let s = &mut *(stream as *mut std::net::TcpStream);
        use std::io::Write;
        match s.write_all(slice) {
            Ok(_) => len as i64,
            Err(_) => -1,
        }
    } else {
        -1
    }
}
#[no_mangle]
pub unsafe extern "C" fn rt_net_receive(stream: i64, max_len: i64) -> i64 {
    if stream <= 0 {
        return rt_string_from_c_str("\0".as_ptr() as *const _);
    }

    let max = if max_len <= 0 { 4096 } else { max_len as usize };
    let s = &mut *(stream as *mut std::net::TcpStream);
    let mut buf = vec![0u8; max];
    use std::io::Read;
    match s.read(&mut buf) {
        Ok(n) => {
            buf.truncate(n);
            buf.push(0);
            rt_string_from_c_str(buf.as_ptr() as *const _)
        }
        Err(_) => rt_string_from_c_str("\0".as_ptr() as *const _),
    }
}
#[no_mangle]
pub unsafe extern "C" fn rt_net_receive_resolver_worker(args: i64) {
    let pid = rt_array_get_fast(args, 0);
    let vec_ptr = rt_array_get_fast(args, 1);

    let res_str = if vec_ptr == 0 {
        rt_string_from_c_str("\0".as_ptr() as *const _)
    } else {
        let boxed_vec: Box<Vec<u8>> = Box::from_raw(vec_ptr as *mut Vec<u8>);
        rt_string_from_c_str(boxed_vec.as_ptr() as *const _)
    };

    rt_promise_resolve(pid, res_str);
}
#[no_mangle]
pub unsafe extern "C" fn rt_net_close(stream: i64) -> i64 {
    if stream <= 0 {
        return -1;
    }
    let _ = Box::from_raw(stream as *mut std::net::TcpStream); // drops & closes
    0
}
