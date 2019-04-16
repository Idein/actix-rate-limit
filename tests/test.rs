use actix_rate_limit_test::*;
use actix_web::{actix, client};
use futures::future::Future;

#[test]
fn test() {
    let test_server = TestServer::new();

    actix::run(|| {
        client::get("http://127.0.0.1:8000")
            .finish()
            .unwrap()
            .send()
            .map(|r| {
                dbg!(r);
                actix::System::current().stop()
            })
            .map_err(|_| ())
    });
}
