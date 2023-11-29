use dsh_sdk::kafka_properties::KafkaProperties;
use dsh_sdk::graceful_shutdown::Shutdown;

use rdkafka::consumer::CommitMode;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::message::BorrowedMessage;
use rdkafka::message::Message;

fn deserialize_and_print(msg: &BorrowedMessage) {
    let payload = match msg.payload() {
        Some(p) => std::string::String::from_utf8_lossy(p),
        None => std::borrow::Cow::Borrowed(""),
    };
    let key = match msg.key() {
        Some(p) => std::string::String::from_utf8_lossy(p),
        None => std::borrow::Cow::Borrowed(""),
    };

    println!(
        "Received message from topic {} partition {} offset {} with key {:?} and payload {}",
        msg.topic(),
        msg.partition(),
        msg.offset(),
        key,
        payload
    );
}

async fn consume(consumer: StreamConsumer, shutdown: Shutdown) {
    loop {
        tokio::select! {
            Ok(msg) = consumer.recv() => {
                    deserialize_and_print(&msg);
                    // Commit the message
                    match consumer.commit_message(&msg, CommitMode::Sync)
                    {
                        Ok(_) => {}
                        Err(e) => println!("Error while committing message: {:?}", e),
                    }
            },
            _ = shutdown.recv() => {
                println!("Shutdown requested, breaking out of consumer");
                consumer.unsubscribe();
                break;
             }
        }
    }
}

#[tokio::main]
async fn main() {
    // Create a new kafka_properties instance (connects to the DSH server and fetches the datastream)
    let kafka_properties = match KafkaProperties::new().await {
        Ok(b) => b,
        Err(e) => {
            println!("Error bootstrap to DSH: {:?}", e);
            return;
        }
    };

    // Get the configured topics from env variable TOPICS (comma separated)
    let topis_string = std::env::var("TOPICS").expect("TOPICS env variable not set");
    let topics = topis_string.split(",").map(|s| s).collect::<Vec<&str>>();

    // Validate your configured topic if it has read access (optional)
    match kafka_properties
        .datastream()
        .verify_list_of_topics(&topics, dsh_sdk::kafka_properties::datastream::ReadWriteAccess::Read)
    {
        Ok(_) => {}
        Err(e) => {
            println!("Error validating topics: {:?}", e);
            return;
        }
    };

    // Initialize the shutdown handler (This will handle SIGTERM and SIGINT signals and you can act on them)
    let shutdown = Shutdown::new();

    // Get the consumer config from the bootstrap instance
    let mut consumer_client_config = kafka_properties.consumer_rdkafka_config();

    // Override some default values (optional)
    consumer_client_config.set("auto.offset.reset", "latest");

    // Create a new consumer instance
    let consumer: StreamConsumer = consumer_client_config
        .create()
        .expect("Consumer creation failed");

    // Subscribe to the configured topics
    consumer
        .subscribe(&topics)
        .expect("Can't subscribe to specified topics");

    // Create a future for consuming messages,
    let consume_future = consume(consumer, shutdown.clone());
    let consumer_handle = tokio::spawn(async move {
        consume_future.await;
    });

    // Wait for shutdown signal or that the consumer has stopped
    tokio::select! {
        _ = shutdown.signal_listener() => {
            println!("Shutdown signal received");
        }
        _ = consumer_handle => {
            println!("Consumer stopped");
            shutdown.start(); // Start the shutdown process (this will stop other potential running tasks, if you create them)
        }
    }

    // Wait till the shutdown is complete
    shutdown.complete().await;
}
