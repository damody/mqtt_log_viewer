use rumqttc::{MqttOptions, AsyncClient, QoS};
use std::time::Duration;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating MQTT test sender...");
    
    let mut mqttoptions = MqttOptions::new("test_sender", "127.0.0.1", 1883);
    mqttoptions.set_keep_alive(Duration::from_secs(5));

    let (client, mut eventloop) = AsyncClient::new(mqttoptions, 10);

    // Send a test message
    tokio::spawn(async move {
        for i in 0..5 {
            let topic = format!("test/topic{}", i);
            let payload = format!("{{\"message_id\": {}, \"temperature\": {}, \"timestamp\": \"{}\"}}", 
                                i, 20 + i, chrono::Utc::now().to_rfc3339());
            
            match client.publish(topic, QoS::AtLeastOnce, false, payload).await {
                Ok(_) => println!("Published message {}", i),
                Err(e) => eprintln!("Failed to publish message {}: {}", i, e),
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    // Handle events
    loop {
        match eventloop.poll().await {
            Ok(notification) => {
                println!("Received = {:?}", notification);
            }
            Err(e) => {
                eprintln!("Error = {:?}", e);
                break;
            }
        }
    }

    Ok(())
}