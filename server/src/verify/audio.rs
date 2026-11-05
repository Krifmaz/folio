//! Audiobook checks: what `ffprobe` finds in the files (length, chapters, tags, language) against the edition.
//! `ffprobe` runs with an argument list, never through a shell.

use super::Check;
use crate::library::text;
use crate::metadata::EditionMeta;
use crate::wanted::store::Profile;
use kit_server_core::{err, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The audio file types Folio reads, by extension.
pub const EXTENSIONS: &[&str] = &["m4b", "m4a", "mp4", "mp3", "aac", "ogg", "oga", "opus", "flac", "wma", "wav"];

/// How far the runtime may be off and still be the same edition: 3 %.
pub const RUNTIME_SLACK: f64 = 0.03;

/// What one audio file says about itself.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioFile {
    /// where it is
    pub path: PathBuf,
    /// its length in milliseconds
    pub duration_ms: u64,
    /// the chapters it carries
    pub chapters: usize,
    /// those chapters: title, start and end in ms from the start of this file
    pub chapter_list: Vec<(String, u64, u64)>,
    /// its tags, by lower-case name
    pub tags: BTreeMap<String, String>,
    /// the audio stream's language tag (`eng`), "" when it has none
    pub language: String,
}

/// What the files of one download say, together.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioFacts {
    /// every file, in name order
    pub files: Vec<AudioFile>,
    /// the length of all of them, in milliseconds
    pub duration_ms: u64,
    /// the chapters: the ones the files carry, or one per file when they carry none and there are several
    pub chapters: usize,
}

impl AudioFacts {
    /// The files' facts added up (`files` in name order).
    pub fn of(mut files: Vec<AudioFile>) -> AudioFacts {
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let duration_ms = files.iter().map(|f| f.duration_ms).sum();
        let carried: usize = files.iter().map(|f| f.chapters).sum();
        let chapters = if carried == 0 && files.len() > 1 { files.len() } else { carried };
        AudioFacts { files, duration_ms, chapters }
    }

    /// The first non-empty value of any of the tags `names`, across the files.
    pub fn tag(&self, names: &[&str]) -> String {
        for f in &self.files {
            for n in names {
                if let Some(v) = f.tags.get(*n).filter(|v| !v.trim().is_empty()) {
                    return v.trim().to_string();
                }
            }
        }
        String::new()
    }

    /// The language the files give, as two letters; "" when none says (or says `und`).
    pub fn language(&self) -> String {
        let raw = self
            .files
            .iter()
            .map(|f| f.language.clone())
            .find(|l| !l.is_empty() && l != "und")
            .unwrap_or_else(|| self.tag(&["language", "lang"]));
        if raw.is_empty() || raw == "und" {
            String::new()
        } else {
            text::language(&raw)
        }
    }
}

/// Reads one file's facts from `ffprobe -print_format json -show_format -show_streams -show_chapters`.
pub fn parse(path: PathBuf, v: &Value) -> AudioFile {
    let secs = v["format"]["duration"].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    let mut tags = BTreeMap::new();
    let audio = v["streams"].as_array().and_then(|s| s.iter().find(|s| s["codec_type"] == "audio")).cloned().unwrap_or(Value::Null);
    for t in [&audio["tags"], &v["format"]["tags"]] {
        if let Some(o) = t.as_object() {
            for (k, val) in o {
                if let Some(s) = val.as_str() {
                    tags.insert(k.to_lowercase(), s.to_string());
                }
            }
        }
    }
    let ms = |c: &Value, k: &str| (c[k].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0) * 1000.0).round() as u64;
    let chapter_list: Vec<(String, u64, u64)> = v["chapters"]
        .as_array()
        .map(|a| {
            a.iter().map(|c| (c["tags"]["title"].as_str().unwrap_or("").to_string(), ms(c, "start_time"), ms(c, "end_time"))).collect()
        })
        .unwrap_or_default();
    AudioFile {
        path,
        duration_ms: (secs * 1000.0).round() as u64,
        chapters: chapter_list.len(),
        chapter_list,
        language: audio["tags"]["language"].as_str().unwrap_or("").to_lowercase(),
        tags,
    }
}

/// Runs `ffprobe` (the setting's path) on each file.
pub async fn probe(ffprobe: &str, paths: &[PathBuf]) -> Result<AudioFacts> {
    let bin = if ffprobe.trim().is_empty() { "ffprobe" } else { ffprobe.trim() };
    let mut files = Vec::new();
    for p in paths {
        let out = tokio::process::Command::new(bin)
            .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams", "-show_chapters"])
            .arg(p)
            .output()
            .await
            .map_err(|e| {
                err(
                    500,
                    format!("Folio can't run {bin} to check the audio ({e}). It comes with Folio's image; elsewhere install ffmpeg or set its path under Settings → Downloads."),
                )
            })?;
        if !out.status.success() {
            let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(err(
                422,
                format!(
                    "{} can't be read as audio: {}",
                    p.display(),
                    if why.is_empty() { "the file is damaged or not audio" } else { &why }
                ),
            ));
        }
        let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
        files.push(parse(p.clone(), &v));
    }
    Ok(AudioFacts::of(files))
}

/// A length in milliseconds as people say it (`"10 h 52 min"`).
fn length(ms: u64) -> String {
    crate::lantern::hours(((ms + 30_000) / 60_000) as u32)
}

/// Whether one of `names` is in `hay` with all its words.
fn names_in(hay: &str, names: &[String]) -> bool {
    let h: Vec<String> = text::fold(hay).split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
    names.iter().any(|n| {
        let w: Vec<String> = text::fold(n).split(' ').filter(|w| w.len() > 1).map(str::to_string).collect();
        !w.is_empty() && w.iter().all(|x| h.contains(x))
    })
}

/// Every check of an audiobook download against the edition `m` and its preferences `pr`.
pub fn checks(m: &EditionMeta, pr: &Profile, f: &AudioFacts) -> Vec<Check> {
    let mut out = Vec::new();
    let got = f.duration_ms;

    // runtime: within 3 %; far shorter is a sample or a part, somewhat shorter may be abridged
    let want = m.runtime_min.map(|r| r as u64 * 60_000).unwrap_or(0);
    let c = Check::new("Runtime", 40, if want > 0 { length(want) } else { String::new() }, length(got)).hard();
    out.push(if want == 0 {
        Check { hard: false, ..c.unknown("The edition doesn't say how long it runs, so the length can't be compared.") }
    } else {
        let ratio = got as f64 / want as f64;
        let off = ((ratio - 1.0).abs() * 100.0).round();
        if (ratio - 1.0).abs() <= RUNTIME_SLACK {
            c.pass(format!("It runs {}, within 3% of the edition's {}.", length(got), length(want)))
        } else if ratio < 0.5 {
            c.fail(format!("It runs only {} of the edition's {}: a sample or part of the book, not all of it.", length(got), length(want)))
        } else if ratio < 1.0 && m.abridged != Some(true) {
            c.fail(format!(
                "It runs {}, {off}% shorter than the edition's {}: likely abridged or missing parts.",
                length(got),
                length(want)
            ))
        } else {
            c.fail(format!("It runs {}, {off}% off the edition's {}: more than 3%, so it is another edition.", length(got), length(want)))
        }
    });

    // the whole book: nothing in the tags says sample, excerpt or abridged
    let words = text::fold(&[f.tag(&["title"]), f.tag(&["album"]), f.tag(&["comment", "description"])].join(" "));
    let w: Vec<&str> = words.split(' ').collect();
    let unabridged_wanted = pr.unabridged_only || m.abridged == Some(false);
    let c = Check::new("Whole book", 0, if unabridged_wanted { "unabridged" } else { "the whole book" }, "").hard();
    out.push(if let Some(x) = ["sample", "excerpt", "preview", "teaser"].iter().find(|x| w.contains(x)) {
        Check { arrived: (*x).into(), ..c.fail(format!("The files call themselves a {x}.")) }
    } else if unabridged_wanted && w.contains(&"abridged") && !w.contains(&"unabridged") {
        Check { arrived: "abridged".into(), ..c.fail("The files say they are abridged; you want the unabridged book.") }
    } else {
        c.pass("Nothing marks it as a sample or abridged.")
    });

    // narrator: one of the edition's narrators is named in the tags
    let tagged = f.tag(&["narrator", "narratedby", "composer", "performer"]);
    let anywhere = [tagged.clone(), f.tag(&["artist"]), f.tag(&["album_artist"]), f.tag(&["comment", "description"])].join(" ");
    let c = Check::new(
        "Narrator",
        20,
        m.narrators.join(", "),
        if tagged.is_empty() { f.tag(&["artist", "album_artist"]) } else { tagged.clone() },
    );
    out.push(if m.narrators.is_empty() {
        c.unknown("The edition doesn't say who reads it.")
    } else if names_in(&anywhere, &m.narrators) {
        c.pass(format!("Read by {}, as the edition says.", m.narrators.join(", ")))
    } else if !tagged.is_empty() {
        c.fail(format!("The files say it is read by {tagged}; the edition is read by {}.", m.narrators.join(", ")))
    } else {
        c.unknown("The files don't say who reads it.")
    });

    // language: the audio's language tag against the edition's (or the preferences')
    let lang = f.language();
    let wanted_lang = if m.language.is_empty() { pr.languages.first().cloned().unwrap_or_default() } else { m.language.clone() };
    let c =
        Check::new("Language", 10, crate::wanted::store::language_name(&wanted_lang), crate::wanted::store::language_name(&lang)).hard();
    out.push(if lang.is_empty() || wanted_lang.is_empty() {
        c.unknown("The files don't say their language.")
    } else if lang == wanted_lang {
        c.pass(format!("In {}.", crate::wanted::store::language_name(&lang)))
    } else {
        c.fail(format!(
            "It is in {}; the edition is in {}.",
            crate::wanted::store::language_name(&lang),
            crate::wanted::store::language_name(&wanted_lang)
        ))
    });

    // chapters: the count matches the edition's, give or take 2 (opening and end credits)
    let n = m.chapters.len();
    let c = Check::new(
        "Chapters",
        20,
        if n > 0 { n.to_string() } else { String::new() },
        if f.chapters > 0 { f.chapters.to_string() } else { String::new() },
    );
    out.push(if n == 0 {
        c.unknown("The edition has no chapter list to compare with.")
    } else if f.chapters == 0 {
        c.unknown("The file has no chapters to count.")
    } else if f.chapters.abs_diff(n) <= 2 {
        c.pass(format!("{} chapters, as the edition has {n}.", f.chapters))
    } else {
        c.fail(format!("It has {} chapters; the edition has {n}.", f.chapters))
    });

    // title: the album or title tag names the book
    let t = f.tag(&["album", "title"]);
    let short = m.title.split(':').next().unwrap_or(&m.title).to_string();
    let c = Check::new("Title", 10, m.title.clone(), t.clone());
    out.push(if t.is_empty() {
        c.unknown("The files don't give a title.")
    } else if names_in(&t, std::slice::from_ref(&short)) {
        c.pass("The title matches.")
    } else {
        c.fail(format!("The files are titled \"{t}\", not \"{}\".", m.title))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{Chapter, Format};

    fn edition() -> EditionMeta {
        EditionMeta {
            format: Format::Audiobook,
            title: "Project Hail Mary".into(),
            narrators: vec!["Ray Porter".into()],
            language: "en".into(),
            runtime_min: Some(970),
            abridged: Some(false),
            chapters: (0..10).map(|i| Chapter { title: format!("Chapter {i}"), start_ms: i * 1000, length_ms: 1000 }).collect(),
            ..Default::default()
        }
    }

    fn profile() -> Profile {
        Profile {
            id: 1,
            format: Format::Audiobook,
            name: "A".into(),
            file_formats: vec!["m4b".into()],
            languages: vec!["en".into()],
            unabridged_only: true,
        }
    }

    fn file(ms: u64, chapters: usize, tags: &[(&str, &str)], lang: &str) -> AudioFacts {
        AudioFacts::of(vec![AudioFile {
            path: "a.m4b".into(),
            duration_ms: ms,
            chapters,
            tags: tags.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            language: lang.into(),
            ..Default::default()
        }])
    }

    fn outcome(c: &[Check], name: &str) -> super::super::Outcome {
        c.iter().find(|c| c.name == name).unwrap().outcome
    }

    #[test]
    fn the_right_file_passes_everything() {
        let f = file(970 * 60_000 + 90_000, 11, &[("album", "Project Hail Mary"), ("composer", "Ray Porter")], "eng");
        let c = checks(&edition(), &profile(), &f);
        assert!(c.iter().all(|c| c.outcome == super::super::Outcome::Pass), "{c:#?}");
        assert_eq!(super::super::decide(c, 85).confidence, 100);
    }

    #[test]
    fn runtime_off_by_more_than_3_percent_or_a_sample_is_wrong() {
        let c = checks(&edition(), &profile(), &file(970 * 60_000 * 9 / 10, 10, &[], ""));
        let r = c.iter().find(|c| c.name == "Runtime").unwrap();
        assert!(r.hard && r.why.contains("abridged"), "{}", r.why);
        let c = checks(&edition(), &profile(), &file(5 * 60_000, 1, &[("title", "Project Hail Mary (Sample)")], ""));
        assert!(c.iter().find(|c| c.name == "Runtime").unwrap().why.contains("sample"));
        assert_eq!(outcome(&c, "Whole book"), super::super::Outcome::Fail);
    }

    #[test]
    fn another_narrator_or_language_fails_and_missing_tags_are_unknown() {
        let c = checks(&edition(), &profile(), &file(970 * 60_000, 10, &[("composer", "Someone Else")], "deu"));
        assert_eq!(outcome(&c, "Narrator"), super::super::Outcome::Fail);
        assert_eq!(outcome(&c, "Language"), super::super::Outcome::Fail);
        let c = checks(&edition(), &profile(), &file(970 * 60_000, 0, &[], ""));
        for n in ["Narrator", "Language", "Chapters", "Title"] {
            assert_eq!(outcome(&c, n), super::super::Outcome::Unknown, "{n}");
        }
        assert_eq!(super::super::decide(c, 85).decision, "review");
    }

    #[test]
    fn several_files_without_chapters_count_one_chapter_each() {
        let files = (0..3).map(|i| AudioFile { path: format!("{i:02}.mp3").into(), duration_ms: 1000, ..Default::default() }).collect();
        let f = AudioFacts::of(files);
        assert_eq!((f.duration_ms, f.chapters), (3000, 3));
    }

    #[test]
    fn ffprobe_json_is_read() {
        let v: Value = serde_json::json!({
            "format": {"duration": "61.500", "tags": {"ALBUM": "Book", "composer": "Ray Porter"}},
            "streams": [{"codec_type": "audio", "tags": {"language": "eng"}}],
            "chapters": [{"start_time": "0.000", "end_time": "30.000", "tags": {"title": "One"}}, {"start_time": "30.000", "end_time": "61.500"}]
        });
        let f = parse("x.m4b".into(), &v);
        assert_eq!((f.duration_ms, f.chapters, f.language.as_str()), (61_500, 2, "eng"));
        assert_eq!(f.tags.get("album").map(String::as_str), Some("Book"));
        assert_eq!(f.chapter_list[0], ("One".to_string(), 0, 30_000));
    }
}
