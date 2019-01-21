extern crate redis_async;
extern crate actix_redis;
extern crate actix_web;
extern crate mdo;
extern crate futures;
extern crate mdo_future;
extern crate redis;

use actix_redis::{Command, RedisActor, RespValue};
use redis_async::resp_array;
use actix_web::error::ErrorInternalServerError;
use actix_web::middleware::{Middleware, Started};
use actix_web::HttpRequest;
use futures::{Future, IntoFuture};
use mdo::mdo;
use mdo_future::future::{bind, ret};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct LeakyBucket<S> {
    redis: actix_web::actix::Addr<RedisActor>,
    access_interval_msec: usize,
    burst_tolerance: usize,
    identify: Rc<dyn Fn(HttpRequest<S>) -> Result<String, ::actix_web::Error>>,
    script_hash: Vec<u8>,
}

impl<S> LeakyBucket<S> {
    pub fn new(
        redis_url: &str,
        access_interval: Duration,
        burst_tolerance: usize,
        identify: Rc<dyn Fn(HttpRequest<S>) -> Result<String, ::actix_web::Error>>,
    ) -> Self {
        let client = ::redis::Client::open(&format!("redis://{}", redis_url)[..]).unwrap();
        let con = client.get_connection().unwrap();
        let script_hash: Vec<u8> = ::redis::cmd("SCRIPT")
            .arg("LOAD")
            .arg(LUA_SCRIPT.as_bytes())
            .query(&con)
            .unwrap();

        let redis = RedisActor::start(redis_url);
        LeakyBucket {
            redis,
            access_interval_msec: access_interval.as_secs() as usize * 1000
                + access_interval.subsec_millis() as usize,
            burst_tolerance,
            identify,
            script_hash,
        }
    }
}

const LUA_SCRIPT: &str = r#"
local function proc(key, capacity, interval, now)
    local tmp = redis.call('HMGET', key, 'last_release', 'count')
    local last_release = tmp[1]
    local count = tmp[2]

    if last_release == false then
        redis.call('HMSET', key, 'last_release', now)
    else
        local elapsed = now - last_release
        local released = math.floor(elapsed / interval)
        local new_count = math.max(count - released, 0)
        redis.call('HMSET', key, 
            'last_release', last_release + interval * released,
            'count', new_count)
        if new_count > capacity then
            return 0
        end 
    end

    local count = redis.call('HINCRBY', key, 'count', 1)
    redis.call('PEXPIRE', key, count * interval)

    return 1
end 

return proc(KEYS[1], tonumber(ARGV[1]), tonumber(ARGV[2]), tonumber(ARGV[3]))
"#;

impl<S: 'static> Middleware<S> for LeakyBucket<S> {
    fn start(&self, req: &HttpRequest<S>) -> Result<Started, ::actix_web::Error> {
        let burst_tolerance = self.burst_tolerance;
        let access_interval_msec = self.access_interval_msec;
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ErrorInternalServerError("invalid system time"))?;
        let epoch_ms: usize = epoch.as_secs() as usize * 1000 + epoch.subsec_millis() as usize;

        let redis = self.redis.clone();
        let id = (self.identify)(req.clone())?;
        let key = format!("ratelimit:id:{}", id);
        let script_hash = self.script_hash.clone();

        let fut = mdo! {
            let v = resp_array![
                "EVALSHA",
                script_hash,
                1.to_string(),
                key,
                burst_tolerance.to_string(),
                access_interval_msec.to_string(),
                epoch_ms.to_string()
            ];
            let command = Command(v);
            resp =<< redis.send(command).map_err(Into::into);
            ret match resp {
                Ok(RespValue::Integer(n)) =>
                    if n == 0 {
                        use ::actix_web::error::InternalError;
                        use ::actix_web::http::StatusCode;
                        Err(InternalError::new(
                                "rate limit exceeded", StatusCode::TOO_MANY_REQUESTS).into())
                            .into_future()
                    } else {
                        ret(None)
                    },
                Ok(_) => Err(ErrorInternalServerError(format!("redis:  {:?}", resp))).into_future(),
                Err(e) => Err(ErrorInternalServerError(format!("redis:  {:?}", e))).into_future(),
            }
        };
        Ok(Started::Future(Box::new(fut)))
    }
}
