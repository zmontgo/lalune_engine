use log::{info, error};
use env_logger::Env;
use dotenv::dotenv;
use sqlx::PgPool;
use tokio_stream::wrappers::ReceiverStream;
use futures_util::stream::StreamExt;
use bb8::Pool;
use bb8_redis::RedisConnectionManager;

mod fitbit;
mod cache;
mod database;
mod errors;
mod models;
mod utils;

// TODO
// - [ ] Implement refresh token request
// - [ ] Implement communication across redis
// - [ ] Implement Fitbit Errors
// - [ ] Refactor to not use tuples for return values
// - [ ] Don't use 'as' for type conversions
// - [ ] Switch timestamps to ISO strings for memory efficiency

#[tokio::main]
async fn main() {
  dotenv().ok();

  let env = Env::default()
    .filter_or("LOG_LEVEL", "trace")
    .write_style_or("LOG_STYLE", "always");

  env_logger::init_from_env(env);

  let redis_pool = cache::CacheHandler::build_pool().await;
  let database_pool = database::DatabaseHandler::build_pool().await;
  
  let mut command_stream = cache::CacheHandler::get_stream(&redis_pool).await;

  info!("Listening for redis stream...");

  match listen(&mut command_stream, redis_pool, database_pool).await {
    Ok(_) => info!("Stream terminated"),
    Err(e) => error!("Error: {:?}", e),
  }
}



async fn listen<'a>(command_stream: &mut ReceiverStream<String>, redis_pool: Pool<RedisConnectionManager>, database_pool: PgPool) -> Result<(), Box<dyn std::error::Error>> {  
  let reqwest_client = reqwest::Client::new();
  
  let cache_client = cache::CacheHandler::new(redis_pool);
  let database_client = database::DatabaseHandler::new(database_pool);
  
  let fitbit_client = fitbit::Fitbit::new(
    reqwest_client,
    cache_client,
    database_client,
  );

  command_stream.for_each_concurrent(None, move |message| {
    let fitbit_client = fitbit_client.clone();

    tokio::spawn(async move {
      info!("Received message: {:?}", message);

      let Some(message) = utils::decode_message(message) else {
        info!("Error decoding message");
        return futures_util::future::ready(())
      };

      info!("Message parsed: {:?}", message);

      let coordination_id = message.0;
      let command = match message.1 {
        Ok(command) => command,
        Err(e) => {
          fitbit_client.reply(coordination_id, models::Response::Error(e)).await;
          return futures_util::future::ready(())
        },
      };
    
      let reply = fitbit_client.execute_command(command).await;

      info!("Sending reply: {:?}", reply);
      
      fitbit_client.reply(coordination_id, reply).await;
      
      futures_util::future::ready(())
    });

    futures_util::future::ready(())
  }).await;

  Ok(())
}