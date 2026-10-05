use truhabit_api::{AppState, MIGRATOR, config::Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env().map_err(|e| format!("Configuration: {e}"))?;
    let pool = truhabit_api::connect(&config).await.map_err(
        |_| "Cannot connect to PostgreSQL. Check configuration; credentials were not logged.",
    )?;
    if std::env::args().any(|arg| arg == "migrate") {
        MIGRATOR
            .run(&pool)
            .await
            .map_err(|_| "Migration failed. Inspect schema and deployment configuration.")?;
        println!("PostgreSQL migrations completed.");
        return Ok(());
    }
    // Runtime credentials have no DDL privileges; migration is an explicit separate operation.
    sqlx::query("SELECT id FROM users LIMIT 0")
        .execute(&pool)
        .await
        .map_err(|_| "Run migrations before starting the application")?;
    let bind = config.bind;
    let origin = config.origin.clone();
    let state = AppState::new(pool, config)
        .await
        .map_err(|_| "Cannot initialize authentication")?;
    let worker_state = state.clone();
    let worker = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        let mut ticks = 0_u32;
        loop {
            interval.tick().await;
            ticks = ticks.wrapping_add(1);
            if let Err(_error) = truhabit_api::mail::deliver_one(&worker_state).await {
                eprintln!("Mail worker operation failed.");
            }
            if ticks.is_multiple_of(60) && truhabit_api::mail::cleanup(&worker_state).await.is_err()
            {
                eprintln!("Retention cleanup failed.");
            }
        }
    });
    let dist =
        std::path::PathBuf::from(std::env::var("DIST_DIR").unwrap_or_else(|_| "web/dist".into()));
    if !dist.join("index.html").is_file() {
        return Err("Frontend missing: build web/dist or configure DIST_DIR".into());
    }
    let app = truhabit_api::router(state, dist);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!("TruHabit: {origin}");
    println!(
        "PostgreSQL accounts and goals active. Real payments and live activity integrations unavailable."
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    worker.abort();
    Ok(())
}
