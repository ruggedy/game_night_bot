use anyhow::Result;
use chrono::Duration;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::io::{self, AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use tokio::time::{Sleep, sleep};
use wacore::pair;
use wacore::pair_code::PairCodeOptions;
use wacore::proto_helpers::MessageExt;
use wacore::types::events::Event;
use waproto::whatsapp::message::{ExtendedTextMessage, PollCreationMessage, poll_creation_message};
use waproto::whatsapp::{self, ContextInfo};
use whatsapp_rust::bot::Bot;
use whatsapp_rust::store::SqliteStore;
use whatsapp_rust::{Client, Jid, SendOptions};
use whatsapp_rust_tokio_transport::TokioWebSocketTransportFactory;
use whatsapp_rust_ureq_http_client::UreqHttpClient;

#[derive(Debug)]
enum SystemEvent {
    WhatsApp(Event),
    Terminal(String),
}

#[derive(Debug)]
pub enum TerminalCommand {
    AllMemberPoll,
    AllMemberRandomGroup { size: usize },
    Poll { name: String, options: Vec<String> },
    Invalid(String),
}

/// Splits a string into tokens, respecting double quotes
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in input.chars() {
        match c {
            '"' => in_quotes = !in_quotes, // Toggle quote state
            ' ' if !in_quotes => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

pub fn parse_command(input: &str) -> Option<TerminalCommand> {
    let tokens = tokenize(input.trim());
    if tokens.is_empty() || !tokens[0].starts_with('!') {
        return None;
    }

    let cmd_name = &tokens[0];
    match cmd_name.as_str() {
        "!all-member-poll" => Some(TerminalCommand::AllMemberPoll),

        "!all-member-random-group" => {
            let mut size = 4; // Default value if no flag is provided
            for token in &tokens[1..] {
                if let Some(arg) = token.strip_prefix("--size=") {
                    if let Ok(parsed_size) = arg.parse::<usize>() {
                        size = parsed_size;
                    }
                }
            }
            Some(TerminalCommand::AllMemberRandomGroup { size })
        }
        "!poll" => {
            let mut name = String::new();
            let mut options = Vec::new();

            for token in &tokens[1..] {
                if let Some(arg) = token.strip_prefix("--") {
                    // Split at the first '=' (e.g., name="Match 5")
                    let parts: Vec<&str> = arg.splitn(2, '=').collect();
                    if parts.len() == 2 {
                        let key = parts[0];
                        // Trim any quotes from the value
                        let val = parts[1].trim_matches('"');

                        match key {
                            "name" => name = val.to_string(),
                            "options" => {
                                // Split comma-separated options and trim quotes from each
                                options = val
                                    .split(',')
                                    .map(|s| s.trim_matches('"').to_string())
                                    .collect();
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some(TerminalCommand::Poll { name, options })
        }
        _ => Some(TerminalCommand::Invalid(cmd_name.to_string())),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let backend = Arc::new(SqliteStore::new("whatsapp.db").await?);

    let (tx, mut rx) = mpsc::channel::<SystemEvent>(100);

    let tx_for_bot = tx.clone();

    let mut bot = Bot::builder()
        .with_backend(backend)
        .with_transport_factory(TokioWebSocketTransportFactory::new())
        .with_http_client(UreqHttpClient::new())
        .with_pair_code(PairCodeOptions {
            phone_number: "+2349012078088".to_string(),
            ..Default::default()
        })
        .on_event({
            let tx_bot_clone = tx_for_bot.clone();
            move |event, _client| {
                let tx = tx_bot_clone.clone();
                async move {
                    let _ = tx.send(SystemEvent::WhatsApp(event)).await;
                }
            }
        })
        .build()
        .await?;

    // bot.client().enable_auto_reconnect.update(
    //     std::sync::atomic::Ordering::Relaxed,
    //     std::sync::atomic::Ordering::Relaxed,
    //     |_| true,
    // );
    let mut bot_handle = bot.run().await?;

    let client = bot.client();

    tokio::spawn(async move { handle_terminal_line(tx).await });
    while let Some(event) = rx.recv().await {
        match event {
            SystemEvent::WhatsApp(event) => {
                let client_wa = client.clone();
                process_whatsapp_event(&client_wa, event).await;
            }
            SystemEvent::Terminal(cmd) => {
                let client_cmd = client.clone();
                process_terminal_cmd(&client_cmd, cmd).await;
            }
        }
    }
    Ok(())
}

async fn handle_terminal_line(tx: mpsc::Sender<SystemEvent>) {
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        let _ = tx.send(SystemEvent::Terminal(line)).await;
    }
}

async fn process_terminal_cmd(client: &Arc<Client>, event: String) {
    println!("Enter AllMemberRandomGroup");
    if let Some(command) = parse_command(&event) {
        println!("Command Parsed succesfully");
        match command {
            TerminalCommand::AllMemberPoll => {
                if let Ok(participants) = get_group_participants(client).await {
                    let poll_message = create_poll_message(
                        "Vote For Member".to_string(),
                        participants
                            .iter()
                            .map(|id| format!("@{}", id.user))
                            .collect(),Some(participants)
                    )
                    .await;
                    let _ = send_poll_message(client, poll_message).await;
                };
            }
            TerminalCommand::AllMemberRandomGroup { size } => {
                println!("AllMemberRandomGroup Selected");
                if let Ok(participants) = get_group_participants(client).await {
                    let random_groups = generate_random_groups(&participants, size);

                    for (i, group) in random_groups.iter().enumerate() {
                        let _ = send_message(
                            client,
                            format!("Group {}", i + 1),
                            Some(group.to_owned()),
                        )
                        .await;

                        sleep(Duration::seconds(3).to_std().unwrap()).await;
                    }
                }
            }
            TerminalCommand::Poll { name, options } => {
                let poll_message = create_poll_message(name, options,None).await;
                let _ = send_poll_message(client, poll_message).await;
            }
            TerminalCommand::Invalid(_) => todo!(),
        }
    }
}

async fn process_whatsapp_event(client: &Arc<Client>, event: Event) {
    match event {
        Event::Connected(_) => {
            // let _ = start_group_actions(&client).await;
            // let my_user = Jid::lid(PERSONAL_JID);

            // let _ = send_message(
            //     &client,
            //     "Test User Message".to_string(),
            //     Some(vec![my_user]),
            // )
            // .await;

            // let poll_message = create_poll_message(
            //     "Test Poll".to_string(),
            //     vec!["Option_one".to_string(), "Option_two".to_string()],
            // )
            // .await;

            // println!("poll message created");
            // // let _ = send_poll_message(&client, poll_message).await;
            // println!("Poll message sent");
            println!("client connected")
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
}

// Hardcoding the JID and my JID for the app.

const GROUP_JID: &str = "120363423352824290";
const PERSONAL_JID: &str = "198093152755924";
const SERVER_URL: &str = "g.us";

use rand::rng;
use rand::seq::SliceRandom; // Needs 'rand' crate in Cargo.toml

pub fn generate_random_groups<T: Clone>(participants: &[T], group_size: usize) -> Vec<Vec<T>> {
    let mut rng = rng();
    let mut shuffled = participants.to_vec();

    // 1. Shuffle the participants randomly
    shuffled.shuffle(&mut rng);

    // 2. Chunk them into gro,Noneups of N
    // .chunks() handles the "remainder" automatically (last group may be smaller)
    shuffled
        .chunks(group_size)
        .map(|chunk| chunk.to_vec())
        .collect()
}

async fn get_group_participants(client: &Arc<Client>) -> anyhow::Result<Vec<Jid>> {
    client
        .groups()
        .query_info(&Jid::new(GROUP_JID, SERVER_URL))
        .await
        .map(|info| info.participants)
}

async fn create_poll_message(
    name: String,
    options: Vec<String>,
    mentions: Option<Vec<Jid>>,
) -> Option<Box<PollCreationMessage>> {
    let mut poll_creation_message = PollCreationMessage {
        name: Some(name),
        options: options
            .into_iter()
            .map(|option| poll_creation_message::Option {
                option_name: Some(option),

                ..Default::default()
            })
            .collect(),
        selectable_options_count: Some(1),
        ..Default::default()
    };

    if let Some(mentions) = mentions {
        poll_creation_message.context_info = Some(Box::new(ContextInfo {
            mentioned_jid: mentions.iter().map(|mention| mention.to_string()).collect(),
            ..Default::default()
        }));
        println!("Context Added");
    }

    Some(Box::new(poll_creation_message))
}

// async fn create
async fn send_poll_message(
    client: &Arc<Client>,
    poll_message: Option<Box<PollCreationMessage>>,
) -> Result<()> {
    let result = client
        .send_message(
            Jid::new(GROUP_JID, SERVER_URL),
            whatsapp::Message {
                poll_creation_message_v3: poll_message,
                ..Default::default()
            },
        )
        .await;

    match result {
        Ok(success) => println!("Success Branch {}", success),
        Err(err) => println!("Failure Branch {:?}", err),
    }
    // println!("{:?}", poll_message);
    Ok(())
}
async fn send_message(
    client: &Arc<Client>,
    message: String,
    mentions: Option<Vec<Jid>>,
) -> Result<()> {
    let extended_text_message = mentions
        .map(|ids| {
            let formatted_ids = ids
                .iter()
                .map(|id| format!("@{}", id.user))
                .collect::<Vec<_>>()
                .join(" ");
            let formatted_text = format!("{}\n{} ", formatted_ids, message);

            Box::new(ExtendedTextMessage {
                text: Some(formatted_text),
                context_info: Some(Box::new(ContextInfo {
                    mentioned_jid: ids.iter().map(|id| id.to_string()).collect(),
                    ..Default::default()
                })),
                ..Default::default()
            })
        })
        .or(Some(Box::new(ExtendedTextMessage {
            text: Some(message),
            ..Default::default()
        })));

    let final_message = whatsapp::Message {
        extended_text_message,
        ..Default::default()
    };

    client
        .send_message(Jid::new(GROUP_JID, SERVER_URL), final_message)
        .await?;
    println!("Client sent message successfully");
    Ok(())
}
