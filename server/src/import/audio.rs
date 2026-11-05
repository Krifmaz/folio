//! Making the library's M4B: one chaptered file with the edition's tags and cover. One M4B or M4A is copied
//! across unchanged in its audio (`-c:a copy`). Several files (MP3s, parts) are joined in name order and
//! encoded to AAC. The chapters come from Audnexus when they cover the book; otherwise each file is one
//! chapter. ffmpeg runs with an argument list, never through a shell.

use crate::metadata::{Chapter, EditionMeta};
use kit_server_core::{err, Result};
use std::path::{Path, PathBuf};

/// One chapter of the finished file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FileChapter {
    /// its title
    pub title: String,
    /// where it starts, ms from the start of the file
    pub start_ms: u64,
    /// where it ends, ms from the start of the file
    pub end_ms: u64,
}

/// The chapters for a file of `total_ms` made from `parts` (each file's title and length, in order). Audnexus'
/// chapters are used when they end within 3% of the length; otherwise one chapter per part. A lone part with no
/// usable chapters keeps no chapter list (the file's own stay).
pub fn chapters(m: &EditionMeta, parts: &[(String, u64)], total_ms: u64) -> Vec<FileChapter> {
    let from_audnexus = |c: &[Chapter]| -> Vec<FileChapter> {
        let mut out: Vec<FileChapter> = c
            .iter()
            .map(|c| FileChapter { title: c.title.clone(), start_ms: c.start_ms, end_ms: (c.start_ms + c.length_ms).min(total_ms) })
            .filter(|c| c.start_ms < total_ms)
            .collect();
        if let Some(last) = out.last_mut() {
            last.end_ms = total_ms;
        }
        out
    };
    if let Some(last) = m.chapters.last() {
        let end = last.start_ms + last.length_ms;
        if total_ms > 0 && (end as f64 - total_ms as f64).abs() <= total_ms as f64 * 0.03 {
            return from_audnexus(&m.chapters);
        }
    }
    if parts.len() < 2 {
        return Vec::new();
    }
    let mut at = 0;
    parts
        .iter()
        .enumerate()
        .map(|(i, (title, len))| {
            let c = FileChapter {
                title: if title.trim().is_empty() { format!("Part {}", i + 1) } else { title.trim().to_string() },
                start_ms: at,
                end_ms: at + len,
            };
            at += len;
            c
        })
        .collect()
}

/// Text in an ffmetadata file: `=`, `;`, `#`, `\` and line breaks escaped.
fn meta_escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '=' | ';' | '#' | '\\' => {
                o.push('\\');
                o.push(c);
            }
            '\n' => o.push_str("\\\n"),
            '\r' => {}
            c => o.push(c),
        }
    }
    o
}

/// The edition's tags as an ffmetadata file, with the chapters.
pub fn ffmetadata(m: &EditionMeta, chapters: &[FileChapter]) -> String {
    let authors = m.authors.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", ");
    let mut tags: Vec<(&str, String)> = vec![
        ("title", m.title.clone()),
        ("album", m.title.clone()),
        ("artist", authors.clone()),
        ("album_artist", authors),
        ("composer", m.narrators.join(", ")),
        ("genre", "Audiobook".into()),
        ("date", crate::library::text::year(&m.release_date).map(|y| y.to_string()).unwrap_or_default()),
        ("publisher", m.publisher.clone()),
        ("comment", m.description.clone()),
        ("language", m.language.clone()),
        ("asin", m.asin.clone()),
    ];
    if let Some(s) = m.series.first() {
        tags.push(("series", s.name.clone()));
        tags.push(("series-part", s.position.clone()));
    }
    let mut out = String::from(";FFMETADATA1\n");
    for (k, v) in tags.into_iter().filter(|(_, v)| !v.trim().is_empty()) {
        out.push_str(&format!("{k}={}\n", meta_escape(&v)));
    }
    for c in chapters {
        out.push_str(&format!("[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={}\ntitle={}\n", c.start_ms, c.end_ms, meta_escape(&c.title)));
    }
    out
}

/// A path in an ffmpeg concat list: `'` closed, escaped and reopened.
fn concat_line(p: &Path) -> String {
    format!("file '{}'\n", p.display().to_string().replace('\'', "'\\''"))
}

/// What [`make_m4b`] needs.
pub struct Job<'a> {
    /// ffmpeg's path
    pub ffmpeg: &'a str,
    /// the downloaded audio files, in play order
    pub inputs: &'a [PathBuf],
    /// the tags and chapters ([`ffmetadata`])
    pub metadata: String,
    /// the cover image, when there is one
    pub cover: Option<&'a Path>,
    /// a scratch folder for the lists (removed after)
    pub work: &'a Path,
    /// the finished file
    pub out: &'a Path,
}

/// Runs ffmpeg to make the library's M4B.
pub async fn make_m4b(j: Job<'_>) -> Result<()> {
    std::fs::create_dir_all(j.work).map_err(|e| err(500, format!("The work folder {} can't be made: {e}", j.work.display())))?;
    let meta = j.work.join("metadata.txt");
    std::fs::write(&meta, &j.metadata).map_err(|e| err(500, format!("The tags can't be written: {e}")))?;
    let one = j.inputs.len() == 1;
    let copy = one
        && j.inputs[0]
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e.to_ascii_lowercase().as_str(), "m4b" | "m4a" | "mp4"))
            .unwrap_or(false);
    let mut cmd = tokio::process::Command::new(if j.ffmpeg.trim().is_empty() { "ffmpeg" } else { j.ffmpeg.trim() });
    cmd.args(["-y", "-v", "error", "-nostdin"]);
    if one {
        cmd.arg("-i").arg(&j.inputs[0]);
    } else {
        let list = j.work.join("files.txt");
        std::fs::write(&list, j.inputs.iter().map(|p| concat_line(p)).collect::<String>())
            .map_err(|e| err(500, format!("The file list can't be written: {e}")))?;
        cmd.args(["-f", "concat", "-safe", "0", "-i"]).arg(&list);
    }
    cmd.arg("-i").arg(&meta);
    if let Some(c) = j.cover {
        cmd.arg("-i").arg(c);
    }
    cmd.args(["-map", "0:a", "-map_metadata", "1", "-map_chapters", "1"]);
    if j.cover.is_some() {
        cmd.args(["-map", "2:v", "-c:v", "copy", "-disposition:v:0", "attached_pic"]);
    }
    if copy {
        cmd.args(["-c:a", "copy"]);
    } else {
        cmd.args(["-c:a", "aac", "-b:a", "64k"]);
    }
    cmd.args(["-movflags", "+faststart", "-f", "mp4"]).arg(j.out);
    let out = cmd.output().await.map_err(|e| {
        err(500, format!("Folio can't run ffmpeg to make the audiobook file ({e}). It comes with Folio's image; elsewhere install ffmpeg."))
    })?;
    let _ = std::fs::remove_dir_all(j.work);
    if !out.status.success() {
        let _ = std::fs::remove_file(j.out);
        let why = String::from_utf8_lossy(&out.stderr).trim().chars().take(600).collect::<String>();
        return Err(err(
            500,
            format!("ffmpeg couldn't make the audiobook file: {}", if why.is_empty() { "no reason given" } else { &why }),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audnexus_chapters_are_used_when_they_fit() {
        let m = EditionMeta {
            chapters: vec![
                Chapter { title: "One".into(), start_ms: 0, length_ms: 60_000 },
                Chapter { title: "Two".into(), start_ms: 60_000, length_ms: 60_000 },
            ],
            ..Default::default()
        };
        let c = chapters(&m, &[("a".into(), 121_000)], 121_000);
        assert_eq!(c.len(), 2);
        assert_eq!((c[1].start_ms, c[1].end_ms), (60_000, 121_000), "the last one runs to the end");
        // far off the length: one chapter per file instead
        let c = chapters(&m, &[("".into(), 300_000), ("Part B".into(), 300_000)], 600_000);
        assert_eq!(c.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["Part 1", "Part B"]);
        assert_eq!(c[1].start_ms, 300_000);
        assert!(chapters(&m, &[("a".into(), 600_000)], 600_000).is_empty());
    }

    #[test]
    fn tags_are_escaped() {
        let m = EditionMeta { title: "A=B; C#".into(), ..Default::default() };
        let t = ffmetadata(&m, &[FileChapter { title: "x\ny".into(), start_ms: 0, end_ms: 1 }]);
        assert!(t.contains("title=A\\=B\\; C\\#\n"), "{t}");
        assert!(t.contains("title=x\\\ny"), "{t}");
        assert_eq!(concat_line(Path::new("/d/it's.mp3")), "file '/d/it'\\''s.mp3'\n");
    }
}
