//! The port of the original's audio stream (`objects/Stream.js`, `routers/HlsRouter.js` and the HLS helpers): when
//! a player can't play a book's files as they are, the server converts them with ffmpeg into 6-second pieces
//! (an HLS stream) that the player fetches from `/hls/<session id>/…`.
//!
//! Each stream lives in `<metadata>/streams/<session id>`: the list of files for ffmpeg (`files.txt`), the
//! playlist the player reads (`output.m3u8`, written up front for the whole book) and the pieces
//! (`output-<n>.ts`). The stream tells the listener's apps how far it got every 2 seconds, starts again from
//! another spot when the player jumps far ahead or back, and is removed with its folder when its session closes.
//! Stream folders left from an earlier run are removed when the first stream starts.

use crate::abs::auth::send_status;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// The length of each piece, in seconds.
pub const SEGMENT_LENGTH: f64 = 6.0;
/// How far before the listener's spot the conversion starts, in seconds, so a short jump back still plays.
const MAX_SEEK_BACK: f64 = 30.0;
/// The audio types the copy can't carry, so they are converted to AAC (`mimeTypesToForceAAC`).
const FORCE_AAC_TYPES: [&str; 7] =
    ["audio/flac", "audio/ogg", "audio/x-ms-wma", "audio/x-aiff", "audio/webm", "audio/amr-wb", "audio/x-caf"];
/// The codecs the copy can't carry (`codecsToForceAAC`).
const FORCE_AAC_CODECS: [&str; 4] = ["alac", "ac3", "eac3", "opus"];

/// The open streams, by their folder.
static STREAMS: LazyLock<Mutex<HashMap<PathBuf, Arc<Stream>>>> = LazyLock::new(Default::default);
/// The streams folders already cleared of leftovers since the server started.
static CLEARED: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(Default::default);

/// The `/hls` route. Like the original, it needs no sign-in: the session id in the address is the key.
pub fn router() -> Router<AppState> {
    Router::new().route("/hls/{stream}/{file}", get(stream_file))
}

/// The folder that holds every stream of this server.
pub fn streams_path(st: &AppState) -> PathBuf {
    st.abs_server.metadata_path.join("streams")
}

/// One open stream.
pub struct Stream {
    /// The session id, which is also the stream's id.
    pub id: String,
    /// The listener's id; the stream's news goes to their apps.
    user_id: String,
    /// The stream's folder.
    dir: PathBuf,
    /// The book's tracks, as a session lists them (with `duration`, `metadata.path`, `mimeType`, `codec`).
    tracks: Vec<Value>,
    /// The book's length in seconds.
    duration: f64,
    /// The expanded library item, for the stream's description (`toJSON`).
    library_item: Value,
    /// The ffmpeg program to run.
    ffmpeg: String,
    /// What changes while the stream runs.
    state: Mutex<RunState>,
}

/// What changes while a stream runs.
#[derive(Default)]
struct RunState {
    /// Where the current conversion starts, in seconds of the book.
    start_time: f64,
    /// Convert to AAC even when the files could be copied (set after a copy failed).
    force_aac: bool,
    /// A restart from another spot is under way.
    resetting: bool,
    /// ffmpeg finished the whole book.
    complete: bool,
    /// The listener's apps were told the stream can be played (`stream_open`).
    client_initialized: bool,
    /// The pieces made so far.
    segments: BTreeSet<i64>,
    /// The furthest piece made so far.
    furthest: i64,
    /// Stops the running conversion.
    cancel: Option<watch::Sender<bool>>,
    /// The running conversion.
    task: Option<JoinHandle<()>>,
    /// The stream was closed.
    closed: bool,
}

impl Stream {
    /// Locks the stream's changing state.
    fn state(&self) -> std::sync::MutexGuard<'_, RunState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The address of the playlist for the player.
    pub fn playlist_uri(&self) -> String {
        format!("/hls/{}/output.m3u8", self.id)
    }

    /// The number of pieces of the whole book (`numSegments`).
    fn num_segments(&self) -> i64 {
        num_segments(self.duration)
    }

    /// The stream as the original describes it (`toJSON`), for the admins' list of open sessions.
    pub fn to_json(&self) -> Value {
        let s = self.state();
        json!({
            "id": self.id,
            "userId": self.user_id,
            "libraryItem": self.library_item,
            "episode": null,
            "segmentLength": SEGMENT_LENGTH as i64,
            "playlistPath": self.dir.join("output.m3u8").display().to_string(),
            "clientPlaylistUri": self.playlist_uri(),
            "startTime": s.start_time,
            "segmentStartNumber": segment_start_number(s.start_time),
            "isTranscodeComplete": s.complete,
        })
    }

    /// Sends news about the stream to the listener's apps.
    fn tell(&self, st: &AppState, event: &str, data: &Value) {
        st.sockets.client_emitter(&self.user_id, event, data);
    }
}

/// The number of pieces for a length in seconds: one more for a last partial piece.
fn num_segments(duration: f64) -> i64 {
    let whole = (duration / SEGMENT_LENGTH).floor();
    let n = whole as i64;
    if duration - whole * SEGMENT_LENGTH > 0.0 {
        n + 1
    } else {
        n
    }
}

/// The number of the first piece a conversion starting at the listener's spot makes (`segmentStartNumber`).
fn segment_start_number(start_time: f64) -> i64 {
    if start_time == 0.0 || !start_time.is_finite() {
        return 0;
    }
    ((start_time - MAX_SEEK_BACK).max(0.0) / SEGMENT_LENGTH).floor() as i64
}

/// The playlist of the whole book (`hlsPlaylistGenerator`): 6-second pieces and a shorter last one.
pub fn playlist(duration: f64) -> String {
    let mut lines: Vec<String> = [
        "#EXTM3U",
        "#EXT-X-VERSION:3",
        "#EXT-X-ALLOW-CACHE:NO",
        "#EXT-X-TARGETDURATION:6",
        "#EXT-X-MEDIA-SEQUENCE:0",
        "#EXT-X-PLAYLIST-TYPE:VOD",
    ]
    .map(String::from)
    .to_vec();
    let whole = (duration / SEGMENT_LENGTH).floor();
    let n = whole as i64;
    for i in 0..n {
        lines.push("#EXTINF:6,".into());
        lines.push(format!("output-{i}.ts"));
    }
    let last = duration - whole * SEGMENT_LENGTH;
    if last > 0.0 {
        lines.push(format!("#EXTINF:{},", crate::abs::library::util::js_num(last)));
        lines.push(format!("output-{n}.ts"));
    }
    lines.push("#EXT-X-ENDLIST".into());
    lines.join("\n")
}

/// The number of a track, or `NaN` like the original's missing index.
fn track_index(t: &Value) -> f64 {
    t.get("index").and_then(Value::as_f64).unwrap_or(f64::NAN)
}

/// The length of a track in seconds.
fn track_duration(t: &Value) -> f64 {
    t.get("duration").and_then(Value::as_f64).unwrap_or(0.0)
}

/// The list of files for ffmpeg (`writeConcatFile`), from the track that holds `start_time` on, and where that
/// track starts in the book.
pub fn concat_file(tracks: &[Value], start_time: f64) -> (String, f64) {
    let mut first_index = 0.0;
    let mut first_start = 0.0;
    if start_time > 0.0 {
        let mut end = 0.0;
        if let Some(t) = tracks.iter().find(|t| {
            end += track_duration(t);
            start_time < end
        }) {
            first_start = end - track_duration(t);
            first_index = track_index(t);
        }
    }
    let text = tracks
        .iter()
        .filter(|t| track_index(t) >= first_index)
        .map(|t| {
            let path = t.pointer("/metadata/path").and_then(Value::as_str).unwrap_or_default().replace('\\', "/").replace('\'', "'\\''");
            let duration = t.get("duration").cloned().unwrap_or(Value::Null);
            format!("file '{path}'\nduration {}", crate::abs::library::util::js_numbers(duration))
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (text, first_start)
}

/// A text field of the first track.
fn first_track_str<'a>(tracks: &'a [Value], key: &str) -> Option<&'a str> {
    tracks.first().and_then(|t| t.get(key)).and_then(Value::as_str)
}

/// The file type of the first track, without the dot (`tracksAudioFileType`).
fn file_type(tracks: &[Value]) -> String {
    tracks.first().and_then(|t| t.pointer("/metadata/ext")).and_then(Value::as_str).unwrap_or_default().trim_start_matches('.').to_string()
}

/// Whether the tracks must be converted to AAC rather than copied.
fn needs_aac(tracks: &[Value], force: bool) -> bool {
    force
        || first_track_str(tracks, "mimeType").is_some_and(|m| FORCE_AAC_TYPES.contains(&m))
        || first_track_str(tracks, "codec").is_some_and(|c| FORCE_AAC_CODECS.contains(&c))
}

/// The arguments for ffmpeg (each on its own; no shell) for a conversion of the files in `dir/files.txt`.
pub fn ffmpeg_args(dir: &FsPath, tracks: &[Value], start_time: f64, track_start: f64, aac: bool) -> Vec<String> {
    let mut args: Vec<String> = ["-seek_timestamp", "1", "-f", "concat", "-safe", "0"].map(String::from).to_vec();
    let adjusted = (start_time - MAX_SEEK_BACK).max(0.0);
    if adjusted > 0.0 {
        let shifted = ((adjusted - track_start) * 10.0).round() / 10.0;
        args.extend(["-ss".into(), format!("{}s", crate::abs::library::util::js_num(shifted)), "-noaccurate_seek".into()]);
    }
    args.extend(["-i".into(), dir.join("files.txt").display().to_string()]);
    let codec = if aac { "aac" } else { "copy" };
    args.extend(["-loglevel", "error", "-map", "0:a", "-c:a", codec].map(String::from));
    let first = tracks.first();
    let track_codec = first_track_str(tracks, "codec").unwrap_or_default();
    let bit_rate = first.and_then(|t| t.get("bitRate")).filter(|v| crate::abs::settings::truthy(Some(v)));
    let channels = first.and_then(|t| t.get("channels")).filter(|v| crate::abs::settings::truthy(Some(v)));
    if let (true, Some(b), Some(c)) = (["ac3", "eac3"].contains(&track_codec), bit_rate, channels) {
        args.extend(["-b:a".into(), crate::abs::library::util::js_numbers(b.clone()).to_string(), "-ac".into(), c.to_string()]);
    }
    args.extend(
        [
            "-f",
            "hls",
            "-copyts",
            "-avoid_negative_ts",
            "make_non_negative",
            "-max_delay",
            "5000000",
            "-max_muxing_queue_size",
            "2048",
            "-hls_time",
            "6",
            "-hls_segment_type",
            "mpegts",
        ]
        .map(String::from),
    );
    args.extend(["-start_number".into(), segment_start_number(start_time).to_string()]);
    args.extend(["-hls_playlist_type", "vod", "-hls_list_size", "0", "-hls_allow_cache", "0"].map(String::from));
    args.extend(["-hls_segment_filename".into(), dir.join("output-%d.ts").display().to_string()]);
    args.extend(["-y".into(), dir.join("final-output.m3u8").display().to_string()]);
    args
}

/// Whether a folder name is a session id (a version 4 UUID), as the leftover cleanup checks.
fn is_v4_uuid(name: &str) -> bool {
    crate::abs::sessions::is_uuid(name) && name.as_bytes()[14] == b'4' && matches!(name.as_bytes()[19], b'8' | b'9' | b'a' | b'b')
}

/// Removes the stream folders of sessions that are not open (`removeOrphanStreams`), once per server run.
fn clear_leftovers(st: &AppState, root: &FsPath) {
    if !CLEARED.lock().unwrap_or_else(|p| p.into_inner()).insert(root.to_path_buf()) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if is_v4_uuid(&name) && crate::abs::playback::open_session(st, &name).is_none() {
            tracing::debug!("Removing the leftover stream folder {}", e.path().display());
            if let Err(err) = std::fs::remove_dir_all(e.path()) {
                tracing::warn!("Couldn't remove the leftover stream folder {}: {err}", e.path().display());
            }
        }
    }
}

/// What a new stream is made from.
pub struct NewStream<'a> {
    /// The session id.
    pub session_id: &'a str,
    /// The listener's id.
    pub user_id: &'a str,
    /// The book's title, the stream track's title.
    pub title: Value,
    /// The book's length in seconds.
    pub duration: f64,
    /// The book's tracks.
    pub tracks: Vec<Value>,
    /// The expanded library item.
    pub library_item: Value,
    /// Where the listener is, in seconds.
    pub start_time: f64,
}

/// Opens a stream for a new session and starts converting (`new Stream`, `generatePlaylist`, `start`). Answers
/// the one track the player plays: the playlist.
pub fn open(st: &AppState, new: NewStream<'_>) -> kit_server_core::Result<Value> {
    let root = streams_path(st);
    std::fs::create_dir_all(&root)
        .map_err(|e| kit_server_core::err(500, format!("The streams folder {} could not be made: {e}", root.display())))?;
    clear_leftovers(st, &root);
    let dir = root.join(new.session_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| kit_server_core::err(500, format!("The stream folder {} could not be made: {e}", dir.display())))?;
    std::fs::write(dir.join("output.m3u8"), playlist(new.duration))
        .map_err(|e| kit_server_core::err(500, format!("The stream's playlist in {} could not be written: {e}", dir.display())))?;
    let ffmpeg = st.settings.str("ffmpeg");
    let stream = Arc::new(Stream {
        id: new.session_id.to_string(),
        user_id: new.user_id.to_string(),
        dir: dir.clone(),
        tracks: new.tracks,
        duration: new.duration,
        library_item: new.library_item,
        ffmpeg: if ffmpeg.trim().is_empty() { "ffmpeg".into() } else { ffmpeg.trim().to_string() },
        state: Mutex::new(RunState { start_time: new.start_time, ..Default::default() }),
    });
    STREAMS.lock().unwrap_or_else(|p| p.into_inner()).insert(dir, stream.clone());
    tracing::info!("Starting the stream {} of {} pieces", stream.id, stream.num_segments());
    start_run(st, &stream);
    Ok(json!({
        "index": 1,
        "startOffset": 0,
        "duration": new.duration,
        "title": new.title,
        "contentUrl": stream.playlist_uri(),
        "mimeType": "application/vnd.apple.mpegurl",
        "codec": null,
        "metadata": null,
    }))
}

/// The open stream with this id.
pub fn get_stream(st: &AppState, id: &str) -> Option<Arc<Stream>> {
    STREAMS.lock().unwrap_or_else(|p| p.into_inner()).get(&streams_path(st).join(id)).cloned()
}

/// Closes a stream (`Stream.close`): stops ffmpeg, removes the folder and tells the listener's apps that it
/// closed, or why it failed.
pub fn close(st: &AppState, id: &str, error: Option<String>) {
    let Some(stream) = STREAMS.lock().unwrap_or_else(|p| p.into_inner()).remove(&streams_path(st).join(id)) else { return };
    let task = {
        let mut s = stream.state();
        s.closed = true;
        if let Some(c) = s.cancel.take() {
            let _ = c.send(true);
        }
        s.task.take()
    };
    tracing::info!("Closing the stream {id}");
    let st = st.clone();
    let finish = async move {
        if let Some(t) = task {
            let _ = t.await;
        }
        match tokio::fs::remove_dir_all(&stream.dir).await {
            Ok(()) => tracing::info!("Removed the stream folder {}", stream.dir.display()),
            Err(e) => tracing::error!("Couldn't remove the stream folder {}: {e}", stream.dir.display()),
        }
        match error {
            Some(e) => stream.tell(&st, "stream_error", &json!({"id": stream.id, "error": e.trim()})),
            None => stream.tell(&st, "stream_closed", &json!(stream.id)),
        }
    };
    match tokio::runtime::Handle::try_current() {
        Ok(h) => {
            h.spawn(finish);
        }
        Err(_) => tracing::warn!("The stream {id} was closed outside the server's runtime; its folder stays until the next start"),
    }
}

/// Starts a conversion from the stream's current start time.
fn start_run(st: &AppState, stream: &Arc<Stream>) {
    let (tx, rx) = watch::channel(false);
    let task = tokio::spawn(run(st.clone(), stream.clone(), rx));
    let mut s = stream.state();
    s.cancel = Some(tx);
    s.task = Some(task);
}

/// Starts again from another spot (`reset`): stops the running conversion, then converts from `time`.
async fn reset(st: &AppState, stream: &Arc<Stream>, time: f64) {
    let task = {
        let mut s = stream.state();
        if s.resetting || s.closed {
            tracing::info!("The stream {} is already starting again", stream.id);
            return;
        }
        s.resetting = true;
        if let Some(c) = s.cancel.take() {
            let _ = c.send(true);
        }
        s.task.take()
    };
    if let Some(t) = task {
        if tokio::time::timeout(Duration::from_secs(10), t).await.is_err() {
            tracing::error!("The stream {}'s conversion didn't stop in time", stream.id);
        }
    }
    {
        let mut s = stream.state();
        if s.closed {
            return;
        }
        s.complete = false;
        s.start_time = time.max(0.0);
    }
    tracing::info!("The stream {} starts again at {}s", stream.id, time.max(0.0));
    start_run(st, stream);
}

/// Checks which pieces exist and tells the listener's apps (`checkFiles`).
async fn check_files(st: &AppState, stream: &Stream) {
    let Ok(mut entries) = tokio::fs::read_dir(&stream.dir).await else { return };
    let mut found = Vec::new();
    while let Ok(Some(e)) = entries.next_entry().await {
        let name = e.file_name().to_string_lossy().to_string();
        if let Some(n) = name.strip_suffix(".ts").and_then(|b| b.split('-').nth(1)).and_then(|n| n.parse::<i64>().ok()) {
            found.push(n);
        }
    }
    let (open_json, progress) = {
        let mut s = stream.state();
        s.segments.extend(found);
        if s.segments.is_empty() {
            return;
        }
        let mut open_json = false;
        if s.segments.len() > 6 && !s.client_initialized {
            s.client_initialized = true;
            open_json = true;
        }
        let last = s.segments.last().copied().unwrap_or_default();
        if last > s.furthest {
            s.furthest = last;
        }
        let percent = format!("{:.2}%", s.segments.len() as f64 * 100.0 / stream.num_segments().max(1) as f64);
        (open_json, json!({"stream": stream.id, "percent": percent, "chunks": chunks(&s.segments), "numSegments": stream.num_segments()}))
    };
    if open_json {
        tracing::info!("The stream {} is ready to play", stream.id);
        stream.tell(st, "stream_open", &stream.to_json());
    }
    stream.tell(st, "stream_progress", &progress);
}

/// The made pieces as runs: a single number, or `"first-last"` for consecutive pieces.
fn chunks(segments: &BTreeSet<i64>) -> Vec<Value> {
    let mut out = Vec::new();
    let mut run: Vec<i64> = Vec::new();
    let flush = |run: &[i64], out: &mut Vec<Value>| match run {
        [] => {}
        [one] => out.push(json!(one)),
        [first, .., last] => out.push(json!(format!("{first}-{last}"))),
    };
    for &n in segments {
        if run.last().is_some_and(|&l| l + 1 != n) {
            flush(&run, &mut out);
            run.clear();
        }
        run.push(n);
    }
    flush(&run, &mut out);
    out
}

/// Runs one conversion: writes the list of files, runs ffmpeg, reports progress every 2 seconds, and handles
/// the end, a failure (one retry with AAC for MP4 audio) or a stop.
async fn run(st: AppState, stream: Arc<Stream>, mut cancel: watch::Receiver<bool>) {
    let (start_time, force_aac, resetting) = {
        let mut s = stream.state();
        s.furthest = 0;
        (s.start_time, s.force_aac, s.resetting)
    };
    let adjusted = (start_time - MAX_SEEK_BACK).max(0.0);
    let (list, track_start) = concat_file(&stream.tracks, adjusted);
    if let Err(e) = tokio::fs::write(stream.dir.join("files.txt"), list).await {
        tracing::error!("Couldn't write the stream's list of files in {}: {e}", stream.dir.display());
        close(&st, &stream.id, Some("The list of files to play could not be written.".into()));
        return;
    }
    let aac = needs_aac(&stream.tracks, force_aac);
    let args = ffmpeg_args(&stream.dir, &stream.tracks, start_time, track_start, aac);
    tracing::info!("Converting the stream {} with {} {}", stream.id, stream.ffmpeg, args.join(" "));
    let mut child = match tokio::process::Command::new(&stream.ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Couldn't start ffmpeg ({}): {e}", stream.ffmpeg);
            close(&st, &stream.id, Some(format!("The audio could not be converted for playing, because ffmpeg could not be started: {e}")));
            return;
        }
    };
    let mut stderr = child.stderr.take();
    let read_errors = tokio::spawn(async move {
        let mut text = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut text).await;
        }
        text
    });
    if resetting {
        let wait = if aac { 3000 } else { 500 };
        tokio::time::sleep(Duration::from_millis(wait)).await;
        stream.state().resetting = false;
    }
    stream.tell(&st, "stream_progress", &json!({"stream": stream.id, "chunks": [], "numSegments": 0, "percent": "0%"}));
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.tick().await;
    let status = loop {
        tokio::select! {
            status = child.wait() => break status,
            _ = cancel.changed() => {
                let _ = child.kill().await;
                tracing::info!("The stream {}'s conversion was stopped", stream.id);
                return;
            }
            _ = tick.tick() => check_files(&st, &stream).await,
        }
    };
    let errors = read_errors.await.unwrap_or_default();
    match status {
        Ok(s) if s.success() => {
            tracing::info!("The stream {} is fully converted", stream.id);
            let tell_open = {
                let mut s = stream.state();
                s.complete = true;
                !std::mem::replace(&mut s.client_initialized, true)
            };
            if tell_open {
                stream.tell(&st, "stream_open", &stream.to_json());
            }
            stream.tell(&st, "stream_ready", &Value::Null);
        }
        other => {
            let code = other.as_ref().ok().and_then(|s| s.code());
            tracing::error!("ffmpeg failed for the stream {} ({code:?}): {}", stream.id, errors.trim());
            let mp4 = ["mp4", "m4a", "m4b"].contains(&file_type(&stream.tracks).as_str());
            if !aac && mp4 && code == Some(1) && !errors.contains("No such file or directory") {
                tracing::info!("Trying the stream {} again, converting to AAC", stream.id);
                stream.state().force_aac = true;
                let st2 = st.clone();
                let stream2 = stream.clone();
                tokio::spawn(async move { reset(&st2, &stream2, start_time).await });
            } else {
                let last = errors.trim().lines().last().unwrap_or("ffmpeg stopped without saying why").to_string();
                close(&st, &stream.id, Some(format!("The audio could not be converted for playing: {last}")));
            }
        }
    }
}

/// Where to start again when a player asks for a piece that isn't made (`checkSegmentNumberRequest`): a piece
/// before the start, or more than 10 pieces past the furthest made, restarts 5 pieces earlier.
fn reset_time_for(s: &RunState, segment: i64) -> Option<f64> {
    let seg_start = segment as f64 * SEGMENT_LENGTH;
    if segment_start_number(s.start_time) > segment {
        return Some(seg_start);
    }
    if s.complete {
        return None;
    }
    if s.furthest != 0 && segment - s.furthest > 10 {
        return Some(seg_start);
    }
    None
}

/// `GET /hls/:stream/:file` (A0203): a piece or the playlist of an open stream. A piece that isn't made yet may
/// restart the conversion there (telling every app with `stream_reset`); it answers 404 until it exists.
async fn stream_file(State(st): State<AppState>, Path((id, file)): Path<(String, String)>) -> Response {
    let Some(stream) = get_stream(&st, &id) else {
        tracing::error!("The stream \"{id}\" isn't open");
        return send_status(StatusCode::NOT_FOUND);
    };
    if file.is_empty() || file == "." || file == ".." || file.contains(['/', '\\']) {
        tracing::error!("A stream file name that isn't allowed: \"{file}\"");
        return send_status(StatusCode::BAD_REQUEST);
    }
    let ext = FsPath::new(&file).extension().and_then(|e| e.to_str()).unwrap_or_default();
    let content_type = match ext {
        "ts" => "video/mp2t",
        "m3u8" => "application/vnd.apple.mpegurl",
        _ => {
            tracing::error!("A stream file must end in .ts or .m3u8: \"{file}\"");
            return send_status(StatusCode::BAD_REQUEST);
        }
    };
    let path = stream.dir.join(&file);
    match tokio::fs::read(&path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "public, max-age=0")], bytes).into_response(),
        Err(_) => {
            tracing::warn!("The stream file {} isn't there", path.display());
            if ext == "ts" {
                let segment = FsPath::new(&file)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|b| b.split('-').nth(1))
                    .and_then(|n| n.parse::<i64>().ok());
                let restart = {
                    let s = stream.state();
                    if s.resetting {
                        tracing::info!("The stream {id} is starting again");
                        None
                    } else {
                        segment.and_then(|n| reset_time_for(&s, n))
                    }
                };
                if let Some(time) = restart {
                    reset(&st, &stream, time - SEGMENT_LENGTH * 5.0).await;
                    tracing::info!("The stream {id} starts again; telling the apps to go to {time}s");
                    st.sockets.emitter("stream_reset", &json!({"startTime": time, "streamId": id}), None);
                }
            }
            send_status(StatusCode::NOT_FOUND)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_playlist_lists_every_piece_and_a_short_last_one() {
        let p = playlist(14.5);
        assert_eq!(
            p,
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-ALLOW-CACHE:NO\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:0\n\
             #EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:6,\noutput-0.ts\n#EXTINF:6,\noutput-1.ts\n#EXTINF:2.5,\noutput-2.ts\n#EXT-X-ENDLIST"
        );
        assert_eq!(num_segments(14.5), 3);
        assert_eq!(num_segments(12.0), 2);
    }

    #[test]
    fn the_file_list_starts_at_the_track_that_holds_the_spot() {
        let tracks = vec![
            json!({"index": 1, "duration": 100, "metadata": {"path": "/b/one's.mp3"}}),
            json!({"index": 2, "duration": 50.5, "metadata": {"path": "/b/two.mp3"}}),
        ];
        assert_eq!(
            concat_file(&tracks, 0.0),
            ("file '/b/one'\\''s.mp3'\nduration 100\n\nfile '/b/two.mp3'\nduration 50.5".to_string(), 0.0)
        );
        assert_eq!(concat_file(&tracks, 120.0), ("file '/b/two.mp3'\nduration 50.5".to_string(), 100.0));
    }

    #[test]
    fn ffmpeg_gets_a_seek_and_the_first_piece_number() {
        let tracks = vec![json!({"index": 1, "duration": 600, "mimeType": "audio/flac", "codec": "flac", "metadata": {"ext": ".flac"}})];
        assert!(needs_aac(&tracks, false));
        let args = ffmpeg_args(FsPath::new("/s/x"), &tracks, 100.0, 0.0, true);
        let joined = args.join(" ");
        assert!(joined.starts_with("-seek_timestamp 1 -f concat -safe 0 -ss 70s -noaccurate_seek -i /s/x/files.txt"), "{joined}");
        assert!(joined.contains("-c:a aac"));
        assert!(joined.contains("-start_number 11"));
        assert!(joined.ends_with("-hls_segment_filename /s/x/output-%d.ts -y /s/x/final-output.m3u8"));
        assert_eq!(segment_start_number(0.0), 0);
        assert_eq!(segment_start_number(20.0), 0);
    }

    #[test]
    fn made_pieces_are_reported_as_runs() {
        let s: BTreeSet<i64> = [0, 1, 2, 5, 7, 8].into_iter().collect();
        assert_eq!(chunks(&s), vec![json!("0-2"), json!(5), json!("7-8")]);
    }

    #[test]
    fn far_jumps_restart_the_conversion() {
        let s = RunState { start_time: 300.0, furthest: 60, ..Default::default() };
        assert_eq!(reset_time_for(&s, 10), Some(60.0));
        assert_eq!(reset_time_for(&s, 70), None);
        assert_eq!(reset_time_for(&s, 71), Some(426.0));
        assert!(is_v4_uuid("6c1f8d0e-3a52-4b7e-9d1a-2f4e5b6c7d80"));
        assert!(!is_v4_uuid("6c1f8d0e-3a52-1b7e-9d1a-2f4e5b6c7d80"));
    }
}
