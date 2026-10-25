//! One synthetic voice call through hub frames, native frames, speech HTTP and
//! helper playback receipts. No host audio or device effects.
#![allow(clippy::unwrap_used)]

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn speech_service() -> Result<(String, mpsc::Receiver<String>, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/", listener.local_addr()?);
    let (sent, requests) = mpsc::channel(128);
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let sent = sent.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut block = [0; 8192];
                let (head, body) = loop {
                    let count = socket.read(&mut block).await.unwrap();
                    if count == 0 {
                        return;
                    }
                    request.extend_from_slice(&block[..count]);
                    if let Some(at) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..at]).to_string();
                        let length: usize = head
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse().unwrap())
                            })
                            .unwrap();
                        if request.len() >= at + 4 + length {
                            break (head, request[at + 4..at + 4 + length].to_vec());
                        }
                    }
                };
                if !head.starts_with("POST /v1/audio/speech ") {
                    // The test supplies the STT receipt through work_event, while
                    // the real worker stays blocked on this synthetic socket.
                    let _socket = socket;
                    std::future::pending::<()>().await;
                    return;
                }
                let text = serde_json::from_slice::<Value>(&body).unwrap()["text"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                // A deliberately slow synthetic voice: 100 ms per character.
                let pcm = vec![0u8; text.chars().count() * 800 * 2];
                let wav = crate::native::wav(&pcm, 8000, 1).unwrap();
                sent.send(text).await.unwrap();
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nX-Orbit-Speech-Kind: clip\r\nX-Orbit-Speech-Provider: synthetic\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    wav.len()
                );
                socket.write_all(headers.as_bytes()).await.unwrap();
                socket.write_all(&wav).await.unwrap();
            });
        }
    });
    Ok((url, requests, task))
}

async fn clip(call: &mut Call, work: &mut mpsc::Receiver<WorkResult>) -> Result<Value> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                command = call.helper.recv() => {
                    let command = command.context("helper closed")?;
                    if command["command"] == "playback_clip" { return Ok(command); }
                }
                event = work.recv() => call.runtime.work_event(event.context("work closed")?).await?,
            }
        }
    }).await?
}

async fn progress(call: &mut Call, id: &Value, state: &str) -> Result<()> {
    call.runtime
        .native_event(json!({"event":"playback_progress", "v":1,
        "playback_id":id,"state":state,"playback_active":state == "started"}))
        .await
}

async fn answer(call: &mut Call, text: &str) -> Result<()> {
    let turn = call
        .runtime
        .state
        .durable
        .assistant
        .active_request
        .clone()
        .context("active request")?;
    call.runtime
        .hub_event(crate::hub::HubEvent::Frame(
            json!({"event":"chat.completed",
        "id":turn,"data":{"text":text,"status":"completed"}}),
        ))
        .await
}

#[tokio::test]
async fn long_reading_barge_in_keeps_all_audio_and_the_next_answer_plays_to_the_end() -> Result<()>
{
    let mut call = Call::open().await?;
    let (url, mut requests, server) = speech_service().await?;
    call.runtime.settings.hub_url = Some(url);
    let (events, mut work) = mpsc::channel(64);
    call.runtime.work_events = events;
    let _hub = call.runtime.link.outbound.open(64);
    // The first voice turn is answered through the same chat completion as a hub.
    call.turn().await?;
    call.transcribed("Cuéntame sobre el proyecto del aula Norte")
        .await?;
    let long = "La sala Norte está lista para la reunión. ".repeat(30);
    assert!(long.chars().count() as u64 * 100 > 60_000);
    answer(&mut call, &long).await?;
    let first = clip(&mut call, &mut work).await?;
    progress(&mut call, &first["playback_id"], "started").await?;
    let _first_text = requests.recv().await.context("first synthesis")?;
    call.frames_over_answer(1, true).await?;
    let mut onset = call_frame(&call.stream, call.sequence, true);
    mark_frame(&mut onset, call.sequence, true);
    onset["playback_active"] = json!(true);
    call.sequence += 1;
    let start = std::time::Instant::now();
    let (handled, paused) = tokio::join!(call.runtime.native_event(onset), async {
        let order = tokio::time::timeout(Duration::from_secs(1), call.helper.recv())
            .await?
            .context("pause")?;
        Ok::<_, anyhow::Error>((order, start.elapsed()))
    });
    handled?;
    let (pause, elapsed) = paused?;
    assert_eq!(pause["command"], "playback_pause");
    assert!(
        elapsed < Duration::from_millis(100),
        "pause dispatch {elapsed:?}"
    );
    eprintln!("call path detected onset to synthetic helper pause: {elapsed:?}");
    progress(&mut call, &first["playback_id"], "paused").await?;
    call.frames(2, true).await?;
    call.frames(8, false).await?;
    // Repeated joins while STT is in flight retain unique PCM from the first
    // voiced frame and its pre-roll, rather than just similarly sized audio.
    for _ in 0..3 {
        call.turn().await?;
    }
    let mut expected = Vec::new();
    for sequence in 10..call.sequence - 1 {
        let voiced = sequence % 12 < 4;
        let mut frame = call_frame(&call.stream, sequence, voiced);
        mark_frame(&mut frame, sequence, voiced);
        expected.extend(crate::capture::frame_pcm(&frame)?);
    }
    let accepted = call
        .transcribed("Espera, añade también el aula Sur")
        .await?;
    let pcm = call.voice_pcm(&accepted)?;
    assert_eq!(
        pcm, expected,
        "each pre-roll and spoken sample must arrive exactly once"
    );
    assert!(
        call.helper_orders()
            .iter()
            .any(|order| order == "playback_stop")
    );
    progress(&mut call, &first["playback_id"], "cancelled").await?;
    assert!(call.runtime.assistant.read_aloud.is_none());
    answer(&mut call, &long).await?;
    let mut heard = String::new();
    let mut duration_ms = 0;
    while call.runtime.assistant.read_aloud.is_some() {
        let command = clip(&mut call, &mut work).await?;
        let text = requests.recv().await.context("next synthesis")?;
        let wav = STANDARD.decode(command["audio_base64"].as_str().context("WAV")?)?;
        let clip_ms = (wav.len() - 44) as u64 * 1000 / 16_000;
        assert!(clip_ms < 60_000);
        duration_ms += clip_ms;
        heard.push_str(&text);
        progress(&mut call, &command["playback_id"], "started").await?;
        progress(&mut call, &command["playback_id"], "played").await?;
    }
    assert_eq!(heard, long);
    assert!(duration_ms > 60_000);
    assert!(
        requests.try_recv().is_err(),
        "cancelled reading queued another part"
    );
    server.abort();
    Ok(())
}

#[tokio::test]
async fn an_unusually_long_provider_clip_is_split_again_before_advancing() -> Result<()> {
    let mut call = Call::open().await?;
    let (url, mut requests, server) = speech_service().await?;
    call.runtime.settings.hub_url = Some(url);
    let (events, mut work) = mpsc::channel(64);
    call.runtime.work_events = events;
    let _hub = call.runtime.link.outbound.open(64);
    call.turn().await?;
    call.transcribed("Lee el siguiente punto").await?;
    let text = "Un resumen sintético del aula Norte. ".repeat(12);
    answer(&mut call, &text).await?;
    let rejected = clip(&mut call, &mut work).await?;
    let first_text = requests.recv().await.context("synthesis")?;
    call.runtime
        .native_event(json!({"event":"response", "v":1,
        "id":rejected["id"], "_bridge_command":"playback_clip", "ok":false,
        "error":{"code":"clip_too_long", "message":"synthetic duration refusal"}}))
        .await?;
    let mut heard = String::new();
    let first_retry = clip(&mut call, &mut work).await?;
    let retry_text = requests.recv().await.context("shorter synthesis")?;
    assert!(retry_text.chars().count() <= first_text.chars().count() / 2);
    heard.push_str(&retry_text);
    progress(&mut call, &first_retry["playback_id"], "played").await?;
    while call.runtime.assistant.read_aloud.is_some() {
        let next = clip(&mut call, &mut work).await?;
        heard.push_str(&requests.recv().await.context("synthesis part")?);
        progress(&mut call, &next["playback_id"], "played").await?;
    }
    assert_eq!(heard, text, "rejected audio is never counted as heard");
    server.abort();
    Ok(())
}

#[tokio::test]
async fn noise_resumes_the_same_reading_without_synthesizing_it_again() -> Result<()> {
    let mut call = Call::open().await?;
    let (url, mut requests, server) = speech_service().await?;
    call.runtime.settings.hub_url = Some(url);
    let (events, mut work) = mpsc::channel(64);
    call.runtime.work_events = events;
    let _hub = call.runtime.link.outbound.open(64);
    call.turn().await?;
    call.transcribed("Lee el resumen del aula Norte").await?;
    answer(&mut call, &"La sala Norte está lista. ".repeat(30)).await?;
    let first = clip(&mut call, &mut work).await?;
    let _text = requests.recv().await.context("synthesis")?;
    let turn_id = call
        .runtime
        .state
        .durable
        .assistant
        .turn
        .as_ref()
        .context("turn")?
        .user
        .id
        .clone();
    let turns = call
        .runtime
        .state
        .durable
        .voice
        .call
        .as_ref()
        .context("call")?
        .turns;
    call.frames_over_answer(2, true).await?;
    assert!(
        call.helper_orders()
            .iter()
            .any(|name| name == "playback_pause")
    );
    progress(&mut call, &first["playback_id"], "paused").await?;
    call.frames(2, true).await?;
    call.frames(8, false).await?;
    call.transcribed("").await?;
    let orders = call.helper_orders();
    assert!(orders.iter().any(|name| name == "playback_resume"));
    assert!(!orders.iter().any(|name| name == "playback_stop"));
    let reading = call
        .runtime
        .assistant
        .read_aloud
        .as_ref()
        .context("reading lost")?;
    assert_eq!(json!(reading.playback_id), first["playback_id"]);
    assert_eq!(
        call.runtime
            .state
            .durable
            .assistant
            .turn
            .as_ref()
            .context("turn")?
            .user
            .id,
        turn_id
    );
    assert_eq!(
        call.runtime
            .state
            .durable
            .voice
            .call
            .as_ref()
            .context("call")?
            .turns,
        turns
    );
    assert!(requests.try_recv().is_err(), "noise restarted synthesis");
    progress(&mut call, &first["playback_id"], "resumed").await?;
    call.runtime.command(Command::Interrupt).await?;
    server.abort();
    Ok(())
}
