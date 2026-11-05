//! Reading an audio file's streams, tags and chapters with ffprobe, like the original's `utils/prober.js`, and
//! making the original's `AudioFile` from it (`AudioFileScanner.scan`, `AudioFile.setDataFromProbe`).

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{LazyLock, RwLock};

/// ffprobe answers to use instead of running ffprobe, by file path (see [`use_recorded`]).
static RECORDED: LazyLock<RwLock<Option<HashMap<String, Value>>>> = LazyLock::new(|| RwLock::new(None));

/// Makes the prober answer with recorded ffprobe output instead of running ffprobe. The parity tests use the
/// original's own ffprobe answers this way, so that a different ffprobe version on the test machine doesn't
/// change durations.
pub fn use_recorded(answers: HashMap<String, Value>) {
    if let Ok(mut r) = RECORDED.write() {
        *r = Some(answers);
    }
}

/// Runs ffprobe on a file (`nodeFfprobe`): its JSON answer, or a plain sentence when it fails.
pub fn raw_probe(path: &str) -> Result<Value, String> {
    if let Some(v) = RECORDED.read().ok().and_then(|r| r.as_ref().and_then(|m| m.get(path).cloned())) {
        return Ok(v);
    }
    let program = std::env::var("FFPROBE_PATH").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "ffprobe".into());
    let out = Command::new(program)
        .args([
            "-hide_banner",
            "-loglevel",
            "fatal",
            "-show_error",
            "-show_format",
            "-show_streams",
            "-show_programs",
            "-show_chapters",
            "-show_private_data",
            "-print_format",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("ffprobe could not be started: {e}"))?;
    serde_json::from_slice(&out.stdout).map_err(|_| "ffprobe gave an answer that could not be read".to_string())
}

/// `isNaN(v)` for ffprobe's values (numbers, numeric text, or missing).
fn js_nan(v: Option<&Value>) -> bool {
    num(v).is_none()
}

/// `Number(v)` for ffprobe's values, `None` for NaN.
fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => crate::abs::library::js_number(s),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Null => Some(0.0),
        _ => None,
    }
}

/// Whether a value is truthy in JavaScript.
fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(crate::abs::models::book::truthy)
}

/// The first non-blank tag among `names`, matched without regard to case (`tryGrabTags`), trimmed.
fn grab_tag(tags: Option<&Value>, names: &[&str]) -> Option<String> {
    let tags = tags?.as_object()?;
    for name in names {
        let key = tags.keys().find(|k| k.to_lowercase() == name.to_lowercase());
        if let Some(v) = key.and_then(|k| tags[k].as_str()) {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// The bit rate of a stream (`tryGrabBitRate`).
fn bit_rate(stream: &Value, all: &[Value], total: Option<f64>) -> Option<f64> {
    if !js_nan(stream.get("bit_rate")) && truthy(stream.get("bit_rate")) {
        return num(stream.get("bit_rate"));
    }
    let tags = stream.get("tags")?;
    let tag = |names: &[&str]| names.iter().find_map(|n| tags.get(*n).filter(|v| truthy(Some(v))).cloned());
    if let Some(bps) = tag(&["BPS", "BPS-eng", "BPS_eng"]) {
        if let Some(n) = num(Some(&bps)) {
            return Some(n);
        }
    }
    let duration = tag(&["DURATION", "DURATION-eng", "DURATION_eng"]);
    let bytes = tag(&["NUMBER_OF_BYTES", "NUMBER_OF_BYTES-eng", "NUMBER_OF_BYTES_eng"]);
    if let (Some(d), Some(b)) = (duration.as_ref().and_then(|d| num(Some(d))), bytes.as_ref().and_then(|b| num(Some(b)))) {
        let bps = (b * 8.0 / d).floor();
        if bps.is_finite() && bps != 0.0 {
            return Some(bps);
        }
    }
    let kind = stream.get("codec_type").and_then(Value::as_str);
    match (total.filter(|t| *t != 0.0), kind) {
        (Some(total), Some("video")) => {
            let mut estimated = total;
            for s in all {
                if truthy(s.get("bit_rate")) {
                    if let Some(b) = num(s.get("bit_rate")) {
                        estimated -= b;
                    }
                }
            }
            let audio_bigger = all.iter().any(|s| {
                s.get("codec_type").and_then(Value::as_str) == Some("audio")
                    && truthy(s.get("bit_rate"))
                    && num(s.get("bit_rate")).is_some_and(|b| b > estimated)
            });
            Some(if audio_bigger { total } else { estimated })
        }
        (_, Some("audio")) => Some(112000.0),
        _ => Some(0.0),
    }
}

/// One stream, cleaned up (`parseMediaStreamInfo`).
fn stream_info(stream: &Value, all: &[Value], total: Option<f64>) -> Map<String, Value> {
    let s = |k: &str| stream.get(k).filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null);
    let mut info = Map::new();
    info.insert("index".into(), stream.get("index").cloned().unwrap_or(Value::Null));
    info.insert("type".into(), stream.get("codec_type").cloned().unwrap_or(Value::Null));
    info.insert("codec".into(), s("codec_name"));
    info.insert("time_base".into(), s("time_base"));
    info.insert("bit_rate".into(), bit_rate(stream, all, total).map(json_num).unwrap_or(Value::Null));
    info.insert("language".into(), grab_tag(stream.get("tags"), &["language"]).map(Value::String).unwrap_or(Value::Null));
    if let Some(t) = stream.get("tags") {
        info.insert("tags".into(), t.clone());
    }
    let kind = stream.get("codec_type").and_then(Value::as_str);
    if kind == Some("audio") || kind == Some("subtitle") {
        let d = stream.pointer("/disposition/default");
        info.insert("is_default".into(), json!(d == Some(&json!(1)) || d == Some(&json!("1"))));
    }
    if kind == Some("audio") {
        info.insert("channels".into(), s("channels"));
        info.insert("sample_rate".into(), num(stream.get("sample_rate")).map(json_num).unwrap_or(Value::Null));
        let layout = stream.get("channel_layout").filter(|v| truthy(Some(v))).map(|l| {
            let text = match l {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            text.split('(').next().unwrap_or("").to_string()
        });
        info.insert("channel_layout".into(), layout.map(Value::String).unwrap_or(Value::Null));
    }
    info
}

/// A number as JavaScript keeps it (whole numbers without a fraction).
fn json_num(n: f64) -> Value {
    crate::abs::library::util::js_num(n)
}

/// The chapters of a file (`parseChapters`), in start order and numbered from 0.
fn chapters(list: Option<&Value>) -> Vec<Value> {
    let Some(list) = list.and_then(Value::as_array) else { return vec![] };
    let mut out: Vec<(f64, f64, String)> = list
        .iter()
        .map(|c| {
            let mut title = c
                .get("TAG:title")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .or_else(|| c.get("title").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            if title.is_empty() {
                title = c.pointer("/tags/title").and_then(Value::as_str).unwrap_or("").to_string();
            }
            let tb = c
                .get("time_base")
                .and_then(Value::as_str)
                .filter(|t| t.contains('/'))
                .and_then(|t| t.split('/').nth(1))
                .and_then(crate::abs::library::js_number)
                .unwrap_or(1.0);
            let pick = |time: &str, raw: &str| -> f64 {
                let not_null_nan = |v: Option<&Value>| v.is_some_and(|v| !v.is_null()) && num(v).is_some();
                if not_null_nan(c.get(time)) {
                    num(c.get(time)).unwrap_or(0.0)
                } else if not_null_nan(c.get(raw)) {
                    num(c.get(raw)).unwrap_or(0.0) / tb
                } else {
                    0.0
                }
            };
            (pick("start_time", "start"), pick("end_time", "end"), title.trim().to_string())
        })
        .collect();
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out.into_iter()
        .enumerate()
        .map(|(i, (start, end, title))| json!({"start": json_num(start), "end": json_num(end), "title": title, "id": i}))
        .collect()
}

/// The file tags the original reads (`parseTags`), only those set, by the original's names (`file_tag_*`).
fn file_tags(tags: Option<&Value>) -> Map<String, Value> {
    let table: &[(&str, &[&str])] = &[
        ("file_tag_encoder", &["encoder", "tsse", "tss"]),
        ("file_tag_encodedby", &["encoded_by", "tenc", "ten"]),
        ("file_tag_title", &["title", "tit2", "tt2"]),
        ("file_tag_titlesort", &["title-sort", "tsot"]),
        ("file_tag_subtitle", &["subtitle", "tit3", "tt3"]),
        ("file_tag_track", &["track", "trck", "trk"]),
        ("file_tag_disc", &["discnumber", "disc", "disk", "tpos", "tpa"]),
        ("file_tag_album", &["album", "talb", "tal"]),
        ("file_tag_albumsort", &["album-sort", "tsoa"]),
        ("file_tag_artist", &["artist", "tpe1", "tp1"]),
        ("file_tag_artistsort", &["artist-sort", "tsop"]),
        ("file_tag_albumartist", &["albumartist", "album_artist", "tpe2"]),
        ("file_tag_date", &["date", "tyer", "tye"]),
        ("file_tag_composer", &["composer", "tcom", "tcm"]),
        ("file_tag_publisher", &["publisher", "tpub", "tpb"]),
        ("file_tag_comment", &["comment", "comm", "com"]),
        ("file_tag_description", &["description", "desc"]),
        ("file_tag_genre", &["genre", "tcon", "tco"]),
        ("file_tag_series", &["series", "show", "mvnm"]),
        ("file_tag_seriespart", &["series-part", "episode_id", "mvin", "part"]),
        ("file_tag_grouping", &["grouping", "grp1"]),
        ("file_tag_isbn", &["isbn"]),
        ("file_tag_language", &["language", "lang"]),
        ("file_tag_asin", &["asin", "audible_asin"]),
        ("file_tag_itunesid", &["itunes-id"]),
        ("file_tag_podcasttype", &["podcast-type"]),
        ("file_tag_episodetype", &["episode-type"]),
        ("file_tag_originalyear", &["originalyear"]),
        ("file_tag_releasecountry", &["MusicBrainz Album Release Country", "releasecountry"]),
        ("file_tag_releasestatus", &["MusicBrainz Album Status", "releasestatus", "musicbrainz_albumstatus"]),
        ("file_tag_releasetype", &["MusicBrainz Album Type", "releasetype", "musicbrainz_albumtype"]),
        ("file_tag_isrc", &["tsrc", "isrc"]),
        ("file_tag_musicbrainz_trackid", &["MusicBrainz Release Track Id", "musicbrainz_releasetrackid"]),
        ("file_tag_musicbrainz_albumid", &["MusicBrainz Album Id", "musicbrainz_albumid"]),
        ("file_tag_musicbrainz_albumartistid", &["MusicBrainz Album Artist Id", "musicbrainz_albumartistid"]),
        ("file_tag_musicbrainz_artistid", &["MusicBrainz Artist Id", "musicbrainz_artistid"]),
        ("file_tag_overdrive_media_marker", &["OverDrive MediaMarkers"]),
    ];
    let mut out = Map::new();
    if tags.is_none() {
        return out;
    }
    for (key, names) in table {
        if let Some(v) = grab_tag(tags, names) {
            out.insert(key.to_string(), Value::String(v));
        }
    }
    out
}

/// The original's `AudioMetaTags` names for the `file_tag_*` tags, in its order.
const META_TAGS: &[(&str, &str)] = &[
    ("tagAlbum", "file_tag_album"),
    ("tagAlbumSort", "file_tag_albumsort"),
    ("tagArtist", "file_tag_artist"),
    ("tagArtistSort", "file_tag_artistsort"),
    ("tagGenre", "file_tag_genre"),
    ("tagTitle", "file_tag_title"),
    ("tagTitleSort", "file_tag_titlesort"),
    ("tagSeries", "file_tag_series"),
    ("tagSeriesPart", "file_tag_seriespart"),
    ("tagGrouping", "file_tag_grouping"),
    ("tagTrack", "file_tag_track"),
    ("tagDisc", "file_tag_disc"),
    ("tagSubtitle", "file_tag_subtitle"),
    ("tagAlbumArtist", "file_tag_albumartist"),
    ("tagDate", "file_tag_date"),
    ("tagComposer", "file_tag_composer"),
    ("tagPublisher", "file_tag_publisher"),
    ("tagComment", "file_tag_comment"),
    ("tagDescription", "file_tag_description"),
    ("tagEncoder", "file_tag_encoder"),
    ("tagEncodedBy", "file_tag_encodedby"),
    ("tagIsbn", "file_tag_isbn"),
    ("tagLanguage", "file_tag_language"),
    ("tagASIN", "file_tag_asin"),
    ("tagItunesId", "file_tag_itunesid"),
    ("tagPodcastType", "file_tag_podcasttype"),
    ("tagEpisodeType", "file_tag_episodetype"),
    ("tagOverdriveMediaMarker", "file_tag_overdrive_media_marker"),
    ("tagOriginalYear", "file_tag_originalyear"),
    ("tagReleaseCountry", "file_tag_releasecountry"),
    ("tagReleaseType", "file_tag_releasetype"),
    ("tagReleaseStatus", "file_tag_releasestatus"),
    ("tagISRC", "file_tag_isrc"),
    ("tagMusicBrainzTrackId", "file_tag_musicbrainz_trackid"),
    ("tagMusicBrainzAlbumId", "file_tag_musicbrainz_albumid"),
    ("tagMusicBrainzAlbumArtistId", "file_tag_musicbrainz_albumartistid"),
    ("tagMusicBrainzArtistId", "file_tag_musicbrainz_artistid"),
];

/// "3/10" → 3 (`trackNumAndTotal.number`): the whole part of the first number, or `None`.
pub fn number_part(tag: Option<&str>) -> Option<i64> {
    let first = tag?.split('/').next()?;
    let n = if first.trim().is_empty() { Some(0.0) } else { crate::abs::library::js_number(first) }?;
    Some(n.trunc() as i64)
}

/// What probing an audio file found (the original's `MediaProbeData`).
#[derive(Clone, Debug, Default)]
pub struct Probe {
    /// The codec of an attached picture (the cover), if any.
    pub embedded_cover_art: Option<String>,
    /// The container's long name.
    pub format: Value,
    /// The length in seconds.
    pub duration: Value,
    /// The bit rate.
    pub bit_rate: Value,
    /// The audio codec.
    pub codec: Value,
    /// The audio stream's time base.
    pub time_base: Value,
    /// The audio stream's language.
    pub language: Value,
    /// "mono", "stereo"...
    pub channel_layout: Value,
    /// The number of channels.
    pub channels: Value,
    /// The chapters.
    pub chapters: Vec<Value>,
    /// The tags, by the original's `tag*` names (only those set).
    pub meta_tags: Map<String, Value>,
}

/// Probes an audio file (`prober.probe`): `Err` with a plain sentence when ffprobe fails or finds no audio.
pub fn probe(path: &str) -> Result<Probe, String> {
    let raw = raw_probe(path)?;
    if let Some(e) = raw.get("error") {
        return Err(e.get("string").and_then(Value::as_str).unwrap_or("ffprobe could not read the file").to_string());
    }
    let format = raw.get("format").cloned().unwrap_or(json!({}));
    let streams: Vec<Value> = raw.get("streams").and_then(Value::as_array).cloned().ok_or("Probe Failed")?;
    let total = if js_nan(format.get("bit_rate")) { None } else { num(format.get("bit_rate")) };
    let cleaned: Vec<Map<String, Value>> = streams.iter().map(|s| stream_info(s, &streams, total)).collect();
    let mut video = cleaned.iter().find(|s| s.get("type").and_then(Value::as_str) == Some("video")).cloned();
    let audio_streams: Vec<&Map<String, Value>> =
        cleaned.iter().filter(|s| s.get("type").and_then(Value::as_str) == Some("audio")).collect();
    let audio = match audio_streams.len() {
        0 => None,
        1 => Some(audio_streams[0].clone()),
        _ => Some(audio_streams.iter().find(|a| a.get("is_default") == Some(&json!(true))).unwrap_or(&audio_streams[0]).to_owned().clone()),
    };
    if audio.is_none() && video.is_none() {
        return Err("Invalid media file: no audio or video streams found".into());
    }
    if let (Some(a), Some(v)) = (&audio, &mut video) {
        let ab = a.get("bit_rate").and_then(Value::as_f64);
        let vb = v.get("bit_rate").and_then(Value::as_f64);
        // JavaScript compares a missing bit rate as 0.
        if ab.unwrap_or(0.0) > vb.unwrap_or(0.0) {
            v.insert("bit_rate".into(), total.map(json_num).unwrap_or(Value::Null));
        }
    }
    let mut tags = file_tags(format.get("tags"));
    if format.get("tags").is_none() {
        if let Some(t) = audio.as_ref().and_then(|a| a.get("tags")) {
            tags = file_tags(Some(t));
        }
    }
    let Some(audio) = audio else { return Err("Invalid audio file no audio stream".into()) };
    let embedded = video.as_ref().and_then(|v| v.get("codec")).and_then(Value::as_str).filter(|c| !c.is_empty()).map(String::from);
    let mut meta_tags = Map::new();
    for (name, key) in META_TAGS {
        if let Some(v) = tags.get(*key) {
            meta_tags.insert(name.to_string(), v.clone());
        }
    }
    let audio_bit = audio.get("bit_rate").filter(|v| truthy(Some(v))).cloned();
    let file_bit =
        if js_nan(format.get("bit_rate")) { Value::Null } else { num(format.get("bit_rate")).map(json_num).unwrap_or(Value::Null) };
    Ok(Probe {
        embedded_cover_art: embedded,
        format: Value::String(
            format
                .get("format_long_name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .or_else(|| format.get("name").and_then(Value::as_str))
                .unwrap_or("Unknown")
                .to_string(),
        ),
        duration: if js_nan(format.get("duration")) {
            Value::Null
        } else {
            num(format.get("duration")).map(json_num).unwrap_or(Value::Null)
        },
        bit_rate: audio_bit.unwrap_or(file_bit),
        codec: audio.get("codec").cloned().unwrap_or(Value::Null),
        time_base: audio.get("time_base").cloned().unwrap_or(Value::Null),
        language: audio.get("language").cloned().unwrap_or(Value::Null),
        channel_layout: audio.get("channel_layout").cloned().unwrap_or(Value::Null),
        channels: audio.get("channels").cloned().unwrap_or(Value::Null),
        chapters: chapters(raw.get("chapters")),
        meta_tags,
    })
}

/// The MIME type of an audio format (`AudioFile.mimeType`), MP3's for unknown ones.
pub fn mime_type(format: &str) -> &'static str {
    match format.to_uppercase().as_str() {
        "M4B" | "M4A" | "MP4" => "audio/mp4",
        "OGG" | "OGA" | "OPUS" => "audio/ogg",
        "AAC" => "audio/aac",
        "FLAC" => "audio/flac",
        "WMA" => "audio/x-ms-wma",
        "AIFF" | "AIF" => "audio/x-aiff",
        "WEBM" | "WEBMA" => "audio/webm",
        "MKA" => "audio/x-matroska",
        "AWB" => "audio/amr-wb",
        "CAF" => "audio/x-caf",
        _ => "audio/mpeg",
    }
}

/// An audio file as the original stores it (`AudioFile.toJSON` after `setDataFromProbe`). `index` is set later by
/// the track order; the track and disc numbers from the file name are given by the caller.
pub fn audio_file(library_file: &Value, p: &Probe, track_from_name: Option<i64>, disc_from_name: Option<i64>, now: i64) -> Value {
    let meta = &library_file["metadata"];
    let ext = meta["ext"].as_str().unwrap_or("");
    let track = number_part(p.meta_tags.get("tagTrack").and_then(Value::as_str));
    let disc = number_part(p.meta_tags.get("tagDisc").and_then(Value::as_str));
    json!({
        "index": null,
        "ino": library_file.get("ino").cloned().filter(|v| truthy(Some(v))).unwrap_or(Value::Null),
        "metadata": {
            "filename": meta["filename"], "ext": meta["ext"], "path": meta["path"], "relPath": meta["relPath"],
            "size": meta["size"], "mtimeMs": meta["mtimeMs"], "ctimeMs": meta["ctimeMs"], "birthtimeMs": meta["birthtimeMs"],
        },
        "addedAt": now,
        "updatedAt": now,
        "trackNumFromMeta": track,
        "discNumFromMeta": disc,
        "trackNumFromFilename": track_from_name,
        "discNumFromFilename": disc_from_name,
        "manuallyVerified": false,
        "exclude": false,
        "error": null,
        "format": p.format,
        "duration": p.duration,
        "bitRate": if truthy(Some(&p.bit_rate)) { p.bit_rate.clone() } else { Value::Null },
        "language": p.language,
        "codec": if truthy(Some(&p.codec)) { p.codec.clone() } else { Value::Null },
        "timeBase": p.time_base,
        "channels": p.channels,
        "channelLayout": p.channel_layout,
        "chapters": p.chapters,
        "embeddedCoverArt": p.embedded_cover_art,
        "metaTags": p.meta_tags,
        "mimeType": mime_type(&super::files::format_of(ext)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_probe_answers_like_the_original() {
        let raw = json!({
            "streams": [{"index": 0, "codec_name": "aac", "codec_type": "audio", "sample_rate": "44100", "channels": 1,
                "channel_layout": "mono", "time_base": "1/44100", "bit_rate": "32539", "disposition": {"default": 1},
                "tags": {"language": "und"}}],
            "chapters": [{"id": 1, "time_base": "1/1000", "start": 20000, "start_time": "20.000000", "end": 40000, "end_time": "40.000000", "tags": {"title": "Chapter 2"}},
                         {"id": 0, "time_base": "1/1000", "start": 0, "start_time": "0.000000", "end": 20000, "end_time": "20.000000", "tags": {"title": " Chapter 1 "}}],
            "format": {"format_long_name": "QuickTime / MOV", "duration": "60.000000", "size": "256184", "bit_rate": "34157",
                "tags": {"title": "Ancillary Justice", "ARTIST": "Ann Leckie", "composer": "Adjoa Andoh", "track": "", "date": "2013"}}
        });
        let mut m = HashMap::new();
        m.insert("/x.m4b".to_string(), raw);
        use_recorded(m);
        let p = probe("/x.m4b").unwrap();
        assert_eq!(p.duration, json!(60));
        assert_eq!(p.bit_rate, json!(32539));
        assert_eq!(p.language, json!("und"));
        assert_eq!(
            p.chapters,
            vec![
                json!({"start": 0, "end": 20, "title": "Chapter 1", "id": 0}),
                json!({"start": 20, "end": 40, "title": "Chapter 2", "id": 1})
            ]
        );
        assert_eq!(
            Value::Object(p.meta_tags.clone()),
            json!({"tagArtist": "Ann Leckie", "tagTitle": "Ancillary Justice", "tagDate": "2013", "tagComposer": "Adjoa Andoh"})
        );
        assert_eq!(number_part(Some("3/10")), Some(3));
        assert_eq!(number_part(Some("x")), None);
    }
}
