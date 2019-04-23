use actix_rate_limit::LeakyBucket;
use actix_web::{actix, client, server, App, HttpResponse};
use futures::future::Future;
use futures::stream;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{self, Duration};

fn get_many_future<'a>(
    uri: String,
    n: usize,
) -> impl Future<Item = Vec<client::ClientResponse>, Error = impl std::fmt::Debug> {
    use stream::Stream;
    let requests = std::iter::repeat_with(move || client::get(&uri).finish().unwrap()).take(n);
    stream::futures_unordered(requests.map(client::ClientRequest::send)).collect()
}

#[derive(Debug)]
struct StatusCount {
    ok: usize,
    too_many_requests: usize,
    other: usize,
}

impl StatusCount {
    fn from_responses(responses: &Vec<client::ClientResponse>) -> Self {
        let mut ok = 0;
        let mut too_many_requests = 0;
        let mut other = 0;
        for res in responses {
            match res.status().as_u16() {
                200 => ok += 1,
                429 => too_many_requests += 1,
                _ => other += 1,
            }
        }
        StatusCount {
            ok,
            too_many_requests,
            other,
        }
    }
}

fn sleep_until(until: time::Instant) {
    let sleep_duration = until - time::Instant::now();
    thread::sleep(sleep_duration);
}
#[test]
fn test() {
    let redis_addr = "127.0.0.1:6379";
    let server_addr = "127.0.0.1:8000";
    let access_interval_ms = 100.0;
    let burst_tolerance = 10;
    let _thread_handle = thread::spawn(move || {
        let sys = actix::System::new("server");
        server::new(move || {
            App::new()
                .middleware(LeakyBucket::new(
                    redis_addr,
                    Duration::from_millis(access_interval_ms as u64),
                    burst_tolerance,
                    Rc::new(|_req| Ok(format!("ratelimit"))),
                ))
                .resource("/", |r| r.f(|_| HttpResponse::Ok()))
        })
        .bind(server_addr)
        .unwrap()
        .start();
        sys.run();
    });

    let mut sys = actix::System::new("client");

    let begin = time::Instant::now();

    {
        let resp = sys
            .block_on(get_many_future(
                format!("http://{}", server_addr),
                burst_tolerance + 1,
            ))
            .unwrap();
        let count = StatusCount::from_responses(&resp);
        dbg!(&count);
        assert_eq!(count.ok, burst_tolerance + 1);
        assert_eq!(count.too_many_requests, 0);
        assert_eq!(count.other, 0);
    }

    {
        let resp = sys
            .block_on(get_many_future(format!("http://{}", server_addr), 10))
            .unwrap();
        let count = StatusCount::from_responses(&resp);
        dbg!(&count);
        assert_eq!(count.ok, 0);
        assert_eq!(count.too_many_requests, 10);
        assert_eq!(count.other, 0);
    }

    {
        sleep_until(begin + Duration::from_millis((access_interval_ms * 1.5) as u64));
        let resp = sys
            .block_on(get_many_future(format!("http://{}", server_addr), 10))
            .unwrap();
        let count = StatusCount::from_responses(&resp);
        dbg!(&count);
        assert_eq!(count.ok, 1);
        assert_eq!(count.too_many_requests, 9);
        assert_eq!(count.other, 0);
    }

    {
        sleep_until(begin + Duration::from_millis((access_interval_ms * 5.5) as u64));
        let resp = sys
            .block_on(get_many_future(format!("http://{}", server_addr), 10))
            .unwrap();
        let count = StatusCount::from_responses(&resp);
        dbg!(&count);
        assert_eq!(count.ok, 4);
        assert_eq!(count.too_many_requests, 6);
        assert_eq!(count.other, 0);
    }
}
