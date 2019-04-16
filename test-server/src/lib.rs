use std::sync::mpsc;
use std::thread;
use actix_web::{actix, server, client, App, HttpResponse};

pub struct TestServer {
    pub addr_string: String,
    thread_handle: thread::JoinHandle<()>,
}

impl TestServer {
    pub fn new() -> Self {
        let addr = "127.0.0.1:8000";
        //let (tx, rx) = mpsc::channel();

        let thread_handle = thread::spawn(move || {
            let sys = actix::System::new("server");
            server::new(
                || App::new()
                .resource("/", |r| r.f(|_| HttpResponse::Ok())))
                .bind(addr).unwrap()
                .start();
            sys.run();
        });


        TestServer {
            addr_string: addr.to_string(),
            thread_handle,
        }
    }

}
