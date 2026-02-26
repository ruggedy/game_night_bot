use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use wacore::pair;
use wacore::pair_code::PairCodeOptions;
use wacore::types::events::Event;
use whatsapp_rust::bot::Bot;
use whatsapp_rust::store::SqliteStore;
use whatsapp_rust::{Client, Jid};
use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
use whatsapp_rust_ureq_http_client::UreqHttpClient;

#[tokio::main]
async fn main() -> Result<()> {
    let backend = Arc::new(SqliteStore::new("whatsapp.db").await?);

    let mut bot = Bot::builder()
        .with_backend(backend)
        .with_transport_factory(TokioWebSocketTransportFactory::new())
        .with_http_client(UreqHttpClient::new())
        .with_pair_code(PairCodeOptions {
            phone_number: "+2349012078088".to_string(),
            ..Default::default()
        })
        .on_event(|event, client| async move {
            match event {
                Event::Connected(_) => {
                    let _ = start_group_actions(&client).await;
                }
                Event::PairingCode { code, timeout } => {
                    println!("value of phone number linking code {}", code);
                }
                Event::PairingQrCode { code, .. } => println!("QR:\n{}", code),
                Event::Message(msg, info) => {
                    println!("Message from {}: {:?}", info.source.sender, msg);
                }
                _ => {}
            }
        })
        .build()
        .await?;

    // bot.client().enable_auto_reconnect.update(
    //     std::sync::atomic::Ordering::Relaxed,
    //     std::sync::atomic::Ordering::Relaxed,
    //     |_| true,
    // );

    println!("GETS TO RUN CALL");
    bot.run().await?.await?;

    Ok(())
}

// Hardcoding the JID and my JID for the app.

const GROUP_JID: &str = "120363423352824290";
const PERSONAL_JID: &str = "198093152755924";
const SERVER_URL: &str = "g.us";

async fn start_group_actions(client: &Arc<Client>) -> anyhow::Result<()> {
    if let Ok(info) = client
        .groups()
        .query_info(&Jid::new(GROUP_JID, SERVER_URL))
        .await
    {
        println!("participants id {:?}", info.participants);
    };
    Ok(())
}
