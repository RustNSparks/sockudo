use clap::Parser;
use pushers::{Channel, Config, Pusher};
use sonic_rs::json;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time;

#[derive(Parser, Debug)]
#[command(author, version, about = "Pusher benchmark tool", long_about = None)]
struct Args {
    /// Interval between messages in seconds
    #[arg(short, long, default_value = "0.1")]
    interval: f64,

    /// Number of messages to send (optional, runs indefinitely if not set)
    #[arg(short, long)]
    messages: Option<u64>,

    /// Host to connect to
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    host: String,

    /// App ID
    #[arg(long, default_value = "app-id")]
    app_id: String,

    /// App key
    #[arg(long, default_value = "app-key")]
    app_key: String,

    /// App secret
    #[arg(long, default_value = "app-secret")]
    app_secret: String,

    /// Port to connect to
    #[arg(short, long, default_value = "6001")]
    port: u16,

    /// Use SSL/TLS
    #[arg(short, long)]
    ssl: bool,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Build Pusher config
    let config = Config::builder()
        .app_id(&args.app_id)
        .key(&args.app_key)
        .secret(&args.app_secret)
        .host(&args.host)
        .port(args.port)
        .use_tls(args.ssl)
        .build()?;

    let pusher = Arc::new(Pusher::new(config)?);
    let channel = Channel::from_string("benchmark")?;
    let channels = vec![channel];

    let mut total_messages = 0u64;
    let interval_duration = Duration::from_secs_f64(args.interval);
    let mut interval_timer = time::interval(interval_duration);

    // Skip the first tick (which fires immediately)
    interval_timer.tick().await;

    loop {
        interval_timer.tick().await;

        // Check if we've reached the message limit
        if let Some(max_messages) = args.messages {
            if total_messages >= max_messages {
                println!("Sent: {} messages", total_messages);
                break;
            }
        }

        // Get current timestamp in milliseconds
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis() as u64;

        // Send message
        let data = json!({
            "time": timestamp
        });

        match pusher.trigger(&channels, "timed-message", data, None).await {
            Ok(_) => {
                total_messages += 1;
                if args.verbose {
                    println!("Sent message with time: {}", timestamp);
                }
            }
            Err(e) => {
                eprintln!("Error sending message: {:?}", e);
            }
        }
    }

    Ok(())
}
