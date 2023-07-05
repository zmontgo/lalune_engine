use chrono::{NaiveDateTime, NaiveDate, Utc, Duration};
use redis::{AsyncCommands, RedisError};
use bb8::Pool;
use bb8_redis::RedisConnectionManager;
use tokio_stream::{wrappers::ReceiverStream};
use tokio::sync::mpsc;
use std::env;
use std::collections::HashMap;
use crate::utils;
use crate::errors::FitbitError;
use log::{info, error};

#[derive(Debug, Clone)]
pub struct CacheHandler {
  pool: Pool<RedisConnectionManager>,
}

impl CacheHandler {
  const REDIS_PREFIX: &'static str = "fitbit:";

  pub fn new(pool: Pool<RedisConnectionManager>) -> Self {
    Self {
      pool,
    }
  }

  pub async fn build_pool() -> Pool<RedisConnectionManager> {
    let redis_url: String = env::var("REDIS_URL").expect("REDIS_URL not set");

    let manager = RedisConnectionManager::new(redis_url).unwrap();
    
    let pool = Pool::builder()
      .build(manager)
      .await
      .expect("Failed to create Redis pool");

    pool
  }

  pub async fn get_stream(pool: &Pool<RedisConnectionManager>) -> ReceiverStream<String> {
    let (tx, rx) = mpsc::channel(100);
    let pool = pool.clone();

    tokio::spawn(async move {
      let mut conn = pool.get().await.unwrap();

      loop {
        let data: Option<(String, String)> = match conn.brpop("requests", 0).await {
          Ok(data) => Some(data),
          Err(e) => {
            error!("Error: {:?}", e);
            None
          },
        };

        if let Some(data) = data {
          tx.send(data.1).await.unwrap();
        }
      }
    });

    ReceiverStream::new(rx)
  }

  pub async fn send_message(&self, coordination_id: &str, message: String) -> Result<(), FitbitError> {
    let mut conn: bb8::PooledConnection<'_, RedisConnectionManager> = self.pool.get().await?;

    let result = conn.set_ex(format!("replies:{coordination_id}"), message, 60).await;

    Ok(result?)
  }

  /// Adds a step count to the user's step count set.
  /// 
  /// # Arguments
  /// 
  /// * `user_id` - The user's Fitbit user ID.
  /// * `date` - The date of the step count.
  /// * `steps` - The number of steps.
  /// 
  /// # Returns
  /// 
  /// * `Ok(())` - If the step count was added successfully.
  /// * `Err(e)` - If the step count could not be added.
  pub async fn add_steps(&self, user_id: &str, date: NaiveDate, steps: u32) -> Result<(), FitbitError> {
    let mut conn = self.pool.get().await?;

    let date = NaiveDateTime::new(date, chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()).timestamp();
    let expire = Utc::now().timestamp() + 60 * 60 * 24 * 2;
    let value = format!("{}:{}:{}", steps, date, expire);

    let mut pipe = redis::pipe();

    let result = pipe.atomic()
      .zadd(format!("fitbit_steps:{}", user_id), value, date)
      .expire(format!("fitbit_steps:{}", user_id), 60 * 60 * 24 * 2)
      .query_async(&mut *conn).await;

    Ok(result?)
  }

  /// Gets the longest range of consecutive days for which the user has step counts in the cache.
  /// 
  /// # Arguments
  /// 
  /// * `start_date` - The start date of the range.
  /// * `end_date` - The end date of the range.
  /// 
  /// # Returns
  /// 
  /// * `Vec<(NaiveDateTime, u32)>` - A vector of tuples containing the date and the number of steps for that date.
  /// * `Err(e)` - If the step counts could not be retrieved.
  pub async fn get_steps(&self, user_id: &str, start_date: NaiveDate, end_date: NaiveDate) -> Result<HashMap<NaiveDate, u32>, FitbitError> {
    let mut conn = self.pool.get().await?;
    let mut expired: Vec<String> = Vec::new();
    
    let start_date_timestamp = NaiveDateTime::new(start_date, chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()).timestamp();
    let end_date_timestamp = NaiveDateTime::new(end_date, chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()).timestamp();

    let steps: Vec<String> = match conn.zrangebyscore(format!("fitbit_steps:{}", user_id), start_date_timestamp, end_date_timestamp).await {
      Ok(steps) => steps,
      Err(e) => {
        match e.kind() {
          redis::ErrorKind::TypeError => return Ok(HashMap::new()),
          _ => return Err(FitbitError::RedisError(e)),
        }
      },
    };

    let now: i64 = Utc::now().timestamp();

    let steps: Vec<(u32, i64)> = steps.into_iter().filter_map(| value | {
      let split_values: Vec<&str> = value.split(':').collect();

      let steps = split_values[0].parse::<u32>().unwrap();
      let timestamp = split_values[1].parse::<i64>().unwrap();
      let expire = split_values[2].parse::<i64>().unwrap();

      if expire < now {
        expired.push(value);
        None
      } else {
        Some((steps, timestamp))
      }
    }).collect();

    if !expired.is_empty() {
      let _: usize = match conn.zrem(format!("fitbit_steps:{}", user_id), expired).await {
        Ok(deleted) => {
          info!("{} entries removed from cache [expired]", deleted);
          deleted
        },
        Err(e) => return Err(FitbitError::RedisError(e)),
      };
    }

    let steps = utils::parse_steps(steps);
    let steps = utils::longest_range(start_date, steps);

    Ok(steps)
  }
  
  /// Stores when a user queries the Fitbit API
  /// 
  /// # Arguments
  /// 
  /// * `user_id` - The user's Fitbit user ID.
  /// * `date` - The date of the query.
  /// * `ratelimit_reset` - The seconds until the ratelimit resets.
  /// 
  /// # Returns
  /// 
  /// * `Ok(())` - If the query was stored successfully.
  /// * `Err(e)` - If the query could not be stored.
  pub async fn add_user_query(&self, user_id: &str, date: NaiveDateTime, ratelimit_reset: usize) -> Result<(), FitbitError> {
    let mut conn = self.pool.get().await?;

    let date = date.timestamp();

    let duration: i64 = match ratelimit_reset.try_into() {
      Ok(duration) => duration,
      Err(err) => return Err(FitbitError::TypeConversionError(err.to_string())),
    };

    let reset_datetime = (Utc::now() + Duration::seconds(duration)).timestamp();

    // Buffer in case of latency
    let reset_datetime = reset_datetime - 2;

    let mut pipe = redis::pipe();

    let query = pipe.atomic()
      .set_ex("fitbit_ratelimit_reset", reset_datetime, ratelimit_reset)
      .lpush(format!("fitbit_user_queries:{}", user_id), date)
      .expire(format!("fitbit_user_queries:{}", user_id), ratelimit_reset)
      .query_async(&mut *conn).await;

    Ok(query?)
  }

  /// Gets the last time a user queried the Fitbit API
  /// 
  /// # Arguments
  /// 
  /// * `user_id` - The user's Fitbit user ID.
  /// 
  /// # Returns
  /// 
  /// * `Ok(Some(date))` - The last time the user queried the Fitbit API.
  pub async fn get_last_user_query(&self, user_id: &str) -> Result<Option<NaiveDateTime>, FitbitError> {
    let mut conn = self.pool.get().await?;

    let last_query: i64 = match conn.lindex(format!("fitbit_user_queries:{}", user_id), 0).await {
      Ok(Some(last_query)) => last_query,
      Ok(None) => return Ok(None),
      Err(e) => return Err(FitbitError::RedisError(e)),
    };
    let last_query = NaiveDateTime::from_timestamp_opt(last_query, 0).unwrap();

    Ok(Some(last_query))
  }

  /// Gets the rate limit reset time
  pub async fn get_ratelimit_reset(&self) -> Result<NaiveDateTime, FitbitError> {
    let mut conn = self.pool.get().await?;

    let ratelimit_reset: Result<Option<i64>, RedisError> = conn.get("fitbit_ratelimit_reset").await;

    let ratelimit_reset: i64 = match ratelimit_reset {
      Ok(Some(ratelimit_reset)) => ratelimit_reset,
      Ok(None) => return Ok(NaiveDateTime::from_timestamp_opt(0, 0).unwrap()),
      Err(e) => return Err(FitbitError::RedisError(e)),
    };

    let ratelimit_reset = NaiveDateTime::from_timestamp_opt(ratelimit_reset, 0).unwrap();

    Ok(ratelimit_reset)
  }

  /// Gets the number of queries a user has made to the Fitbit API
  /// As the expiry time is set to the ratelimit reset time, this should be the number of queries the user has made in the last ratelimit reset time.
  /// 
  /// # Arguments
  /// 
  pub async fn get_user_queries(&self, user_id: &str) -> Result<usize, FitbitError> {
    let mut conn = self.pool.get().await?;

    let length: Result<usize, RedisError> = conn.llen(format!("fitbit_user_queries:{}", user_id)).await;

    match length {
      Ok(length) => Ok(length),
      Err(e) => Err(FitbitError::RedisError(e)),
    }
  }
}