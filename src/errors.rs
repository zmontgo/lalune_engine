use std::error::Error;
use std::fmt;
use redis::RedisError;
use bb8::RunError;

/// Errors that can occur when retrieving steps from Fitbit.
#[derive(Debug)]
pub enum FitbitError {
  HttpRequestError(reqwest::Error),
  FitbitApiError(String),
  CacheError(String),
  ExpiredToken,
  RejectedToken,
  ParsingError(String),
  DateOutOfRange(String),
  RateLimitExceeded(String),
  RedisError(redis::RedisError),
  RedisPoolError(bb8::RunError<RedisError>),
  PostgresError(sqlx::Error),
  TypeConversionError(String),
  InvalidMessage(String),
  UserNotFound,
}

impl From<sqlx::Error> for FitbitError {
  fn from(err: sqlx::Error) -> Self {
    FitbitError::PostgresError(err)
  }
}

impl From<redis::RedisError> for FitbitError {
  fn from(err: RedisError) -> Self {
    FitbitError::RedisError(err)
  }
}

impl From<RunError<RedisError>> for FitbitError {
  fn from(err: RunError<RedisError>) -> Self {
    FitbitError::RedisPoolError(err)
  }
}

impl fmt::Display for FitbitError {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      FitbitError::HttpRequestError(err) => write!(f, "HTTP request failed: {err}"),
      FitbitError::FitbitApiError(err) => write!(f, "Fitbit API error: {err}"),
      FitbitError::CacheError(err) => write!(f, "Cache error: {err}"),
      FitbitError::ExpiredToken => write!(f, "Token expired"),
      FitbitError::RejectedToken => write!(f, "Token rejected"),
      FitbitError::ParsingError(err) => write!(f, "Parsing error: {err}"),
      FitbitError::DateOutOfRange(err) => write!(f, "Date out of range: {err}"),
      FitbitError::RateLimitExceeded(err) => write!(f, "Rate limit exceeded: {err}"),
      FitbitError::RedisError(err) => write!(f, "Redis error: {err}"),
      FitbitError::RedisPoolError(err) => write!(f, "Redis pool error: {err}"),
      FitbitError::PostgresError(err) => write!(f, "Postgres error: {err}"),
      FitbitError::TypeConversionError(err) => write!(f, "Type conversion error: {err}"),
      FitbitError::InvalidMessage(err) => write!(f, "Invalid message: {err}"),
      FitbitError::UserNotFound => write!(f, "User not found"),
    }
  }
}

impl Error for FitbitError {
  fn source(&self) -> Option<&(dyn Error + 'static)> {
    match *self {
      FitbitError::HttpRequestError(ref err) => Some(err),
      _ => None,
    }
  }
}
