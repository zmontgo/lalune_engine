use chrono::NaiveDateTime;
use sqlx::{PgPool, postgres::PgPoolOptions };
use std::env;
use crate::{errors::FitbitError, models::DatabaseUser};
use log::info;

#[derive(Debug, Clone)]
pub struct DatabaseHandler {
  pool: PgPool,
}

impl DatabaseHandler {
  pub fn new(pool: PgPool) -> Self {
    Self {
      pool,
    }
  }

  pub async fn build_pool() -> PgPool {
    let database_url = env::var("DATABASE_URL")
      .expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
      .max_connections(5)
      .connect(&database_url)
      .await
      .expect("Failed to connect to Postgres");

    pool
  }

  /// Checks if a user exists in the database.
  /// 
  /// # Arguments
  /// 
  /// * `user_id` - The user's Fitbit user ID.
  /// 
  /// # Returns
  /// 
  /// * `Ok(true)` - If the user exists.
  /// * `Ok(false)` - If the user does not exist.
  /// * `Err(e)` - If the query failed.
  // pub async fn user_exists(&self, user_id: &str) -> Result<bool, FitbitError> {
  //   let mut conn = self.pool.acquire().await?;
  //   let exists = sqlx::query!("SELECT EXISTS(SELECT 1 FROM fitbit_data WHERE id = $1)", user_id)
  //     .fetch_one(&mut conn)
  //     .await?
  //     .exists;

  //   let exists = match exists {
  //     Some(exists) => exists,
  //     None => panic!("Unexpected null value for user_exists"),
  //   };

  //   Ok(exists)
  // }

  /// Gets a user's Fitbit data from the database.
  /// 
  /// # Arguments
  /// 
  /// * `user_id` - The user's Fitbit user ID.
  /// 
  /// # Returns
  /// 
  /// * `Ok(Some(user))` - If the user exists.
  /// * `Ok(None)` - If the user does not exist.
  /// * `Err(e)` - If the query failed.
  pub async fn get_user(&self, user_id: &str) -> Result<Option<DatabaseUser>, FitbitError> {
    let mut conn = self.pool.acquire().await?;

    let user = sqlx::query_as!(DatabaseUser, "SELECT * FROM fitbit_data WHERE id = $1", user_id)
      .fetch_optional(&mut conn)
      .await?;

    Ok(user)
  }

  /// Checks the stored Fitbit token expiry time and returns whether or not it has expired.
  /// 
  /// # Arguments
  /// 
  /// * `user_id` - The user's Fitbit user ID.
  /// 
  /// # Returns
  /// 
  /// * `Ok(Some(true))` - If the token has expired.
  /// * `Ok(Some(false))` - If the token has not expired.
  /// * `Ok(None)` - If the user does not exist.
  /// * `Err(e)` - If the query failed.
  pub async fn user_token_expired(&self, user_id: &str) -> Result<Option<bool>, FitbitError> {
    let mut conn = self.pool.acquire().await?;
    let expired = sqlx::query!("SELECT id, (EXTRACT(EPOCH FROM(fitbit_token_expires_at - now()))::bigint) AS fitbit_token_expires_in FROM fitbit_data WHERE id = $1", user_id)
      .fetch_one(&mut conn)
      .await?;

    let expired = expired.fitbit_token_expires_in.map(|expired| expired < 0);

    Ok(expired)
  }

  /// Updates a user's Fitbit token in the database.
  /// 
  /// # Arguments
  /// 
  /// * `user_id` - The user's Fitbit user ID.
  /// * `access_token` - The user's Fitbit access token.
  /// * `refresh_token` - The user's Fitbit refresh token.
  /// * `expires_at` - The time at which the user's Fitbit access token expires.
  /// 
  /// # Returns
  /// 
  /// * `Ok(())` - If the update was successful.
  /// * `Err(e)` - If the query failed.
  pub async fn update_user_token(&self, user_id: &str, access_token: &str, refresh_token: &str, expires_at: NaiveDateTime) -> Result<(), FitbitError> {
    let mut conn = self.pool.acquire().await?;
    sqlx::query!("UPDATE fitbit_data SET fitbit_access_token = $1, fitbit_refresh_token = $2, fitbit_token_expires_at = $3 WHERE id = $4", access_token, refresh_token, expires_at, user_id)
      .execute(&mut conn)
      .await?;

    Ok(())
  }
}