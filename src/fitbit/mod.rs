mod api;

use chrono::{Utc, NaiveDateTime, NaiveDate};
use log::{info, error};
use crate::utils;
use crate::models::{Period, Range, Command, Response};
use crate::errors::FitbitError;
use crate::cache::CacheHandler;
use crate::database::DatabaseHandler;
use std::collections::HashMap;
use chrono::Duration;
use std::env;

/// The Fitbit API client. This is designed to be cheaply cloneable to allow for multiple requests to be handled concurrently.
#[derive(Clone)]
pub struct Fitbit {
  reqwest_client: reqwest::Client,
  cache_client: CacheHandler,
  database_client: DatabaseHandler,
  client_id: String,
  client_secret: String,
}

impl Fitbit {
  pub fn new(reqwest_client: reqwest::Client, cache_client: CacheHandler, database_client: DatabaseHandler) -> Self {
    let client_id: String = env::var("FITBIT_CLIENT_ID").expect("FITBIT_CLIENT_ID not set");
    let client_secret: String  = env::var("FITBIT_CLIENT_SECRET").expect("FITBIT_CLIENT_SECRET not set");

    Self {
      reqwest_client,
      cache_client,
      database_client,
      client_id: client_id,
      client_secret: client_secret,
    }
  }

  pub async fn reply(&self, coordination_id: ulid::Ulid, response: Response) {
    let coordination_id = coordination_id.to_string();
    let coordination_id = coordination_id.as_str();

    let response = utils::encode_response(response);

    match self.cache_client.send_message(coordination_id, response).await {
      Ok(_) => (),
      Err(e) => error!("Failed to send message to response list: {}", e),
    };
  }

  pub async fn execute_command(&self, command: Command) -> Response {
    let response: Response;

    match command {
      Command::GetSteps(user_id, range) => {
        let user = match self.database_client.get_user(&user_id).await {
          Ok(Some(user)) => user,
          Ok(None) => return Response::Error(FitbitError::UserNotFound),
          Err(e) => return Response::Error(e),
        };

        let steps = match self.get_steps(&user_id, &user.fitbit_user_id, &user.fitbit_access_token, range.start, range.end).await {
          Ok(steps) => steps,
          Err(e) => return Response::Error(e),
        };

        response = Response::Steps(steps);
      },
      Command::RefreshToken(user_id) => {
        match self.refresh_token(&user_id).await {
          Ok(_) => (),
          Err(e) => return Response::Error(e),
        };

        response = Response::Refreshed;
      },
    }

    response
  }

  /// Gets daily step counts from Fitbit within a given range, inclusive.
  /// 
  /// # Arguments
  /// 
  /// * `user_id` - The user's Fitbit user ID.
  /// * `access_token` - The user's Fitbit access token.
  /// * `start` - The start date of the range.
  /// * `end` - The end date of the range.
  /// 
  /// # Returns
  /// 
  /// * `HashMap<NaiveDate, u32>` - A hashmap of dates and their corresponding step counts.
  /// * `FitbitError` - An error if one occurs.
  pub async fn get_steps(&self, user_id: &str, fitbit_user_id: &str, fitbit_access_token: &str, start: NaiveDate, end: NaiveDate) -> Result<HashMap<NaiveDate, u32>, FitbitError> {
    let token_expired = self.check_access_token_expired(user_id).await?;

    let token_expired = token_expired.unwrap_or(false);

    if token_expired {
      self.refresh_token(user_id).await?;
    }

    let cached_steps = self.get_cached_steps(user_id, start, end).await?;
    let last_cache_date: Option<NaiveDate> = cached_steps.keys().max().copied();

    let live_range = match self.get_live_range(user_id, start, end, last_cache_date).await {
      Ok(Some(range)) => range,
      Ok(None) => return Ok(cached_steps),
      Err(e) => return Err(e),
    };

    let difference = (live_range.end - live_range.start).num_days() as f64;

    let mut steps: HashMap<NaiveDate, u32> = cached_steps;

    let requests: u32 = if difference > 364.0 {
      (difference / 364.0).ceil() as u32
    } else {
      1
    };

    let mut days_left = difference as i64;

    for i in 0..requests {
      let start = start + chrono::Duration::days(i64::from(i * 364));
      let end = start + chrono::Duration::days(std::cmp::min(days_left, 364));

      days_left -= 364;

      steps.extend(self.get_steps_for_range(user_id, fitbit_user_id, fitbit_access_token, start, end).await?);
    }

    Ok(steps)
  }

  /// Checks if we know the users's access token has expired.
  /// Of course, this is not a guarantee that it has not expired, but it is nearly always the case.
  /// 
  /// # Arguments
  /// 
  /// * `user_id` - The user's Fitbit user ID.
  /// 
  /// # Returns
  /// 
  /// * `Option<bool>` - `Some(true)` if the access token has expired, `Some(false)` if it has not expired, `None` if we do not know.
  /// * `FitbitError` - An error if one occurs.
  async fn check_access_token_expired(&self, user_id: &str) -> Result<Option<bool>, FitbitError> {
    let expired = self.database_client.user_token_expired(user_id).await?;

    Ok(expired)
  }

  /// Gets daily step counts from Fitbit within the given range, inclusive.
  async fn get_steps_for_range(&self, user_id: &str, fitbit_user_id: &str, fitbit_access_token: &str, start: NaiveDate, end: NaiveDate) -> Result<HashMap<NaiveDate, u32>, FitbitError> {
    if self.check_ratelimit(user_id).await {
      return Err(FitbitError::RateLimitExceeded(format!("Rate limit exceeded")))?;
    }

    if start > end {
      Err(FitbitError::DateOutOfRange("Start date must be before end date.".to_string()))?;
    }

    let now = Utc::now().naive_local().date();

    if end > now {
      info!("End date: {}", end.format("%Y-%m-%d"));
      info!("Now: {}", now.format("%Y-%m-%d"));
      Err(FitbitError::DateOutOfRange("Dates must be UTC and in the past.".to_string()))?;
    }

    let difference = end.signed_duration_since(start);

    let period = match difference.num_days() {
      0 => Period::OneDay,
      1..=6 => Period::OneWeek,
      7..=27 => Period::OneMonth,
      28..=89 => Period::ThreeMonths,
      90..=179 => Period::SixMonths,
      180..=364 => Period::OneYear,
      _ => Err(FitbitError::DateOutOfRange("Date range must be less than one year.".to_string()))?,
    };

    let (steps, headers) = api::get_steps(&self.reqwest_client, fitbit_user_id, fitbit_access_token, end, period).await?;

    // Filters out days that are not in the range.
    let steps = steps.into_iter()
      .filter(|(date, _)| *date >= start && *date <= end)
      .collect();

    self.set_ratelimit(user_id, &headers).await;
    self.cache(user_id, &steps).await?;

    Ok(steps)
  }

  async fn cache(&self, user_id: &str, steps: &HashMap<NaiveDate, u32>) -> Result<(), FitbitError> {
    info!("Cacheing {} steps", steps.len());

    for (date, steps) in steps {
      match self.cache_client.add_steps(user_id, *date, *steps).await {
        Ok(_) => (),
        Err(e) => return Err(FitbitError::CacheError(e.to_string()))?,
      }
    }

    Ok(())
  }

  async fn set_ratelimit(&self, user_id: &str, headers: &reqwest::header::HeaderMap) -> bool {
    // Seconds until the current rate limit window resets.
    let ratelimit_reset = headers.get("fitbit-rate-limit-reset").unwrap().to_str().unwrap().parse::<i64>().unwrap() as usize;

    let date: NaiveDateTime = Utc::now().naive_local();

    self.cache_client.add_user_query(user_id, date, ratelimit_reset).await.is_ok()
  }

  /// Checks whether the current rate limit window has been reached.
  /// Returns true if the rate limit has been reached, false otherwise.
  async fn check_ratelimit(&self, user_id: &str) -> bool {
    let queries = self.cache_client.get_user_queries(user_id);

    let Ok(queries) = queries.await else {
      return true;
    };

    // RATELIMIT: 145 queries per user per hour.
    queries > 145
  }

  /// Takes in the date range to be queried and returns the date range that should be queried from Fitbit, or None if the entire range is already cached.
  /// This function assumes that it will never be passed dates that are in the future.
  /// 
  /// # Arguments
  /// 
  /// * `user_id` - The user's Fitbit user ID.
  /// * `start` - The start date of the range.
  /// * `end` - The end date of the range.
  /// 
  /// # Returns
  /// 
  /// * `Option<(NaiveDate, NaiveDate)>` - The date range that should be queried from Fitbit, or None if the entire range is already cached.
  /// * `FitbitError` - An error if one occurs.
  async fn get_live_range(&self, user_id: &str, range_start: NaiveDate, range_end: NaiveDate, cache_end: Option<NaiveDate>) -> Result<Option<Range>, FitbitError> {
    let Some(cache_end) = cache_end else {
      return Ok(Some(Range { start: range_start, end: range_end } ));
    };

    // Gets the number of queries made by the user since the current rate limit window started.
    let Ok(queries) = self.cache_client.get_user_queries(user_id).await else {
      return Err(FitbitError::CacheError("Error getting user queries.".to_string()));
    };

    // Gets the date of the user's last query.
    let Ok(last_query_datetime) = self.cache_client.get_last_user_query(user_id).await else {
      return Err(FitbitError::CacheError("Error getting last user query.".to_string()));
    };

    // RATELIMIT: 145 queries per user per hour.
    let remaining = 145.0 - queries as f32;

    let current_datetime: NaiveDateTime = Utc::now().naive_local();
    let ratelimit_reset = self.cache_client.get_ratelimit_reset().await.unwrap_or(Utc::now().naive_local());

    let signed_until_ratelimit_reset: i64 = (ratelimit_reset - current_datetime).num_seconds();
    let until_ratelimit_reset: u16 = utils::safe_convert(signed_until_ratelimit_reset);

     if remaining == 0.0 {
      return Err(FitbitError::RateLimitExceeded("Rate limit exceeded.".to_string()));
    }

    let request_period: i64 = (f32::from(until_ratelimit_reset) / remaining).ceil() as i64;

    // This is an estimate of how often we can query Fitbit without exceeding the rate limit.
    let request_period: usize = utils::safe_convert(request_period);

    match last_query_datetime {
      Some(last_query) => {
        let since_last_query: i64 = (current_datetime - last_query).num_seconds();
        let since_last_query: usize = utils::safe_convert(since_last_query);

        if since_last_query < request_period {
          Ok(None)
        } else {
          info!("Last query was {} seconds ago, check if live query is needed", (current_datetime - last_query).num_seconds());

          // If the last cached day is within 2 days of the end day, return the day before and the end day.
          if range_end == current_datetime.date() && (cache_end - range_end).num_days() > -2 {
            info!("Cache is partially up to date, but since the TTL is 2 days, ensure the last 2 days are up to date");
            info!(" Days saved: {}", (cache_end - range_end).num_days());
            // Cache is partially up to date, but since the TTL is 2 days, ensure the last 2 days are up to date.
            return Ok(Some( Range { start: current_datetime.date() - Duration::days(1), end: current_datetime.date() } ))
          } else if range_end == cache_end {
            info!("Cache is up to date");
            return Ok(None);
          }

          info!("Cache is out of date; query from {} to {}", cache_end, range_end);
          Ok(Some( Range { start: cache_end, end: range_end } ))
        }
      },
      None => Ok(Some( Range { start: range_start, end: range_end } )),
    }
  }

  /// Gets daily step counts from the cache within a given range, inclusive.
  /// Will return the longest range possible from the cache, always starting from the start date.
  /// 
  /// # Arguments
  /// 
  /// * `user_id` - The user's Fitbit user ID.
  /// * `start` - The start date of the range.
  /// * `end` - The end date of the range.
  /// 
  /// # Returns
  /// 
  /// * `HashMap<NaiveDate, u32>` - A hashmap of dates and their corresponding step counts.
  /// * `FitbitError` - An error if one occurs.
  async fn get_cached_steps(&self, user_id: &str, start: NaiveDate, end: NaiveDate) -> Result<HashMap<NaiveDate, u32>, FitbitError> {
    let steps = self.cache_client.get_steps(user_id, start, end).await;

    match steps {
      Ok(steps) => Ok(steps),
      Err(_) => Err(FitbitError::CacheError("Error getting cached steps.".to_string()))?,
    }
  }

  /// Refreshes the access token using the refresh token.
  /// 
  /// # Arguments
  /// 
  /// * `refresh_token` - The refresh token associated with the user you want a new access token for.
  /// 
  /// # Returns
  /// 
  /// * `Ok((access_token, refresh_token))` - The new access token and refresh token.
  /// * `Err(FitbitError)` - The error returned by the internal Fitbit API.
  pub async fn refresh_token(&self, user_id: &str) -> Result<(String, String), FitbitError> {
    let user = self.database_client.get_user(user_id).await?;

    let refresh_token = match user {
      Some(user) => user.fitbit_refresh_token,
      None => return Err(FitbitError::UserNotFound),
    };

    let updated_token = api::refresh_token(&self.reqwest_client, refresh_token.as_str(), self.client_id.as_str(), self.client_secret.as_str()).await?;

    let access_token = updated_token.access_token;
    let refresh_token = updated_token.refresh_token;
    let expires_at = Utc::now().naive_local() + Duration::seconds(i64::from(updated_token.expires_in));

    self.database_client.update_user_token(user_id, access_token.as_str(), refresh_token.as_str(), expires_at).await?;

    Ok((access_token, refresh_token))
  }
}