#[macro_use]
extern crate diesel_migrations;

use actix::prelude::*;
use actix_web::{web::Data, *};
use aircraft_db_schema::{get_database_url_from_env, source::secret::Secret};
use aircraft_utils::{
    AircraftError,
    rate_limit::{RateLimit, rate_limiter::RateLimiter},
    request::build_user_agent,
    settings::structs::Settings,
};
use diesel::{
    PgConnection,
    r2d2::{ConnectionManager, Pool},
};
use doku::json::{AutoComments, Formatting};
use reqwest::Client;
use std::{env, sync::Arc, thread};
use tokio::sync::Mutex;

#[actix_web::main]
async fn main() -> io::Result<()> {
    dotenv().ok();
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL is not set in .env file");
    let conn_str = PgPool::new(database_url).await.unwrap();

    env_logger::init();
    let settings = Settings::init().expect("Couldn't initialize settings.");

    let manager = ConnectionManager::<PgConnection>::new(&db_url);
    let pool = Pool::builder()
        .max_size(settings.database.pool_size)
        .build(manager)
        .unwrap_or_else(|_| panic!("Error connecting to {}", db_url));

    let pool2 = pool.clone();
    thread::spawn(move || {
        scheduled_tasks::setup(pool2).expect("Couldn't set up scheduled_tasks");
    });

    // Initialize the secrets
    let conn = pool.get()?;
    let secret = Secret::init(&conn).expect("Couldn't initialize secrets.");

    println!("Starting http server at {}:{}", settings.bind, settings.port);

    let client = Client::builder().user_agent(build_user_agent(&settings)).build()?;

    // let activity_queue = create_activity_queue();

    // Create Http server with websocket support
    let settings_bind = settings.clone();
    HttpServer::new(move || {
        let context = AircraftContext::create(
            pool.clone(),
            chat_server.to_owned(),
            client.clone(),
            activity_queue.to_owned(),
            settings.to_owned(),
            secret.to_owned(),
        );
        let rate_limiter = rate_limiter.clone();
        App::new()
            .wrap(middleware::Logger::default())
            .app_data(Data::new(context))
            .configure(|cfg| api_routes::config(cfg, &rate_limiter))
    })
    .bind((settings_bind.bind, settings_bind.port))?
    .run()
    .await?;

    Ok(())
}
