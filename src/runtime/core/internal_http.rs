use super::*; // Extracted \n
#[no_mangle]
pub unsafe extern "C" fn rt_http_request(url: i64, method: i64, body: i64) -> i64 {
    let url_str = match i64_to_rust_str(url) {
        Some(s) => s,
        None => return rt_string_from_c_str("\0".as_ptr() as *const _),
    };
    let method_str = i64_to_rust_str(method).unwrap_or_else(|| "GET".to_string());
    let body_str = i64_to_rust_str(body);

    let client = reqwest::blocking::Client::new();
    let mut req = match method_str.as_str() {
        "POST" => client.post(&url_str),
        "PUT" => client.put(&url_str),
        "DELETE" => client.delete(&url_str),
        _ => client.get(&url_str),
    };

    if let Some(b) = body_str {
        if !b.is_empty() {
            req = req.body(b);
        }
    }

    let text = match req.send() {
        Ok(resp) => resp.text().unwrap_or_default(),
        Err(_) => String::new(),
    };
    let c_str = std::ffi::CString::new(text).unwrap_or_default();
    rt_string_from_c_str(c_str.as_ptr())
}
#[no_mangle]
pub unsafe extern "C" fn rt_http_request_async(url: i64, method: i64, body: i64) -> i64 {
    let pid = rt_promise_new();

    let url_str = match i64_to_rust_str(url) {
        Some(s) => s,
        None => {
            let empty = rt_string_from_c_str("\0".as_ptr() as *const _);
            rt_promise_resolve(pid, empty);
            return pid;
        }
    };
    let method_str = i64_to_rust_str(method).unwrap_or_else(|| "GET".to_string());
    let body_str = i64_to_rust_str(body);

    unsafe { crate::event_loop::tejx_inc_async_ops() };
    let handle = unsafe { crate::event_loop::tejx_create_global_handle(pid) };

    crate::event_loop::TOKIO_RT.spawn(async move {
        let client = reqwest::Client::new();
        let mut req = match method_str.as_str() {
            "POST" => client.post(&url_str),
            "PUT" => client.put(&url_str),
            "DELETE" => client.delete(&url_str),
            _ => client.get(&url_str),
        };

        if let Some(b) = body_str {
            if !b.is_empty() {
                req = req.body(b);
            }
        }

        match req.send().await {
            Ok(response) => match response.text().await {
                Ok(text) => unsafe {
                    let actual_pid = crate::event_loop::tejx_get_global_handle(handle);
                    let mut task_args = rt_Array_new_fixed(2, 8);
                    rt_array_set_fast(task_args, 0, actual_pid);
                    let boxed = Box::new(text);
                    let ptr = Box::into_raw(boxed) as i64;
                    rt_array_set_fast(task_args, 1, ptr);
                    crate::event_loop::tejx_enqueue_task(
                        rt_http_request_resolver_worker as i64,
                        task_args,
                    );
                },
                Err(_) => unsafe {
                    let actual_pid = crate::event_loop::tejx_get_global_handle(handle);
                    let mut task_args = rt_Array_new_fixed(2, 8);
                    rt_array_set_fast(task_args, 0, actual_pid);
                    rt_array_set_fast(task_args, 1, 0);
                    crate::event_loop::tejx_enqueue_task(
                        rt_http_request_resolver_worker as i64,
                        task_args,
                    );
                },
            },
            Err(_) => unsafe {
                let actual_pid = crate::event_loop::tejx_get_global_handle(handle);
                let mut task_args = rt_Array_new_fixed(2, 8);
                rt_array_set_fast(task_args, 0, actual_pid);
                rt_array_set_fast(task_args, 1, 0);
                crate::event_loop::tejx_enqueue_task(
                    rt_http_request_resolver_worker as i64,
                    task_args,
                );
            },
        }
        unsafe { crate::event_loop::tejx_drop_global_handle(handle) };
        unsafe { crate::event_loop::tejx_dec_async_ops() };
    });

    pid
}
#[no_mangle]
pub unsafe extern "C" fn rt_http_request_resolver_worker(args: i64) {
    let pid = rt_array_get_fast(args, 0);
    let str_ptr = rt_array_get_fast(args, 1);

    let res_str = if str_ptr == 0 {
        rt_string_from_c_str("\0".as_ptr() as *const _)
    } else {
        let boxed_str: Box<String> = Box::from_raw(str_ptr as *mut String);
        let c_str = std::ffi::CString::new(boxed_str.as_str()).unwrap_or_default();
        rt_string_from_c_str(c_str.as_ptr())
    };

    rt_promise_resolve(pid, res_str);
}
