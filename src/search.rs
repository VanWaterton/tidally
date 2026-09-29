//! Search query syntax.
//!
//! `artist <name>`, `album <name>` and `track <name>` can be combined in any order, e.g.
//! `artist radiohead album ok computer`. `artist:name` works too, and quoted text is never
//! treated as a keyword (`album "the artist"`). Anything before the first keyword is free text.

use anyhow::{Result, bail};

use crate::tidal::Client;
use crate::tidal::models::Track;

/// Result limit when fetching extra candidates to filter locally.
const FILTER_POOL: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Free,
    Artist,
    Album,
    Track,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Query {
    pub free: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track: Option<String>,
}

impl Query {
    pub fn parse(input: &str) -> Self {
        let mut q = Query::default();
        let mut field = Field::Free;
        for token in tokenize(input) {
            let mut text = token.text.as_str();
            if !token.quoted
                && let Some((f, rest)) = split_keyword(text)
            {
                field = f;
                text = rest;
            }
            if !text.is_empty() {
                q.push(field, text);
            }
        }
        // A bare keyword like `album` with nothing after it is just a word to search for.
        if q.free.is_empty() && q.is_plain() {
            q.free = input.trim().to_string();
        }
        q
    }

    fn push(&mut self, field: Field, word: &str) {
        let slot = match field {
            Field::Free => &mut self.free,
            Field::Artist => self.artist.get_or_insert_default(),
            Field::Album => self.album.get_or_insert_default(),
            Field::Track => self.track.get_or_insert_default(),
        };
        if !slot.is_empty() {
            slot.push(' ');
        }
        slot.push_str(word);
    }

    pub fn is_plain(&self) -> bool {
        self.artist.is_none() && self.album.is_none() && self.track.is_none()
    }

    /// All terms joined, for Tidal's free-text search.
    fn api_query(&self) -> String {
        [
            self.track.as_deref(),
            self.artist.as_deref(),
            self.album.as_deref(),
            Some(self.free.as_str()),
        ]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
    }
}

pub struct Outcome {
    pub tracks: Vec<Track>,
    /// Describes what the results are, e.g. `Album · OK Computer — Radiohead`.
    pub heading: String,
}

pub async fn run(client: &Client, q: &Query, limit: u32) -> Result<Outcome> {
    if q.is_plain() {
        let tracks = client.search_tracks(&q.free, limit).await?;
        return Ok(Outcome {
            tracks,
            heading: "Tracks".into(),
        });
    }

    // Any `track` filter means the user wants specific songs: search, then filter by every field.
    if let Some(title) = &q.track {
        let tracks = client
            .search_tracks(&q.api_query(), FILTER_POOL)
            .await?
            .into_iter()
            .filter(|t| {
                matches(&t.display_title(), title)
                    && q.artist
                        .as_ref()
                        .is_none_or(|a| matches(&t.artist_names(), a))
                    && q.album.as_ref().is_none_or(|a| matches(t.album_title(), a))
            })
            .take(limit as usize)
            .collect();
        return Ok(Outcome {
            tracks,
            heading: "Tracks".into(),
        });
    }

    if let Some(album_name) = &q.album {
        let albums = client.search_albums(&q.api_query(), FILTER_POOL).await?;
        let album = best_match(
            &albums,
            |a| &a.title,
            album_name,
            |a| {
                q.artist
                    .as_ref()
                    .is_none_or(|artist| matches(&a.artist_names(), artist))
            },
        );
        let Some(album) = album else {
            match &q.artist {
                Some(artist) => bail!("no album “{album_name}” by “{artist}”"),
                None => bail!("no album matching “{album_name}”"),
            }
        };
        let tracks = client.album_tracks(album.id).await?;
        return Ok(Outcome {
            heading: format!("Album · {} — {}", album.title, album.artist_names()),
            tracks,
        });
    }

    let artist_name = q.artist.as_deref().unwrap_or_default();
    let artists = client.search_artists(artist_name, FILTER_POOL).await?;
    let Some(artist) = best_match(&artists, |a| &a.name, artist_name, |_| true) else {
        bail!("no artist matching “{artist_name}”");
    };
    let tracks = client.artist_top_tracks(artist.id, limit).await?;
    Ok(Outcome {
        heading: format!("Top tracks · {}", artist.name),
        tracks,
    })
}

/// Picks an exact (normalized) name match if there is one, otherwise the first partial match.
/// Tidal's own ranking decides ties.
fn best_match<'a, T>(
    items: &'a [T],
    name: impl Fn(&T) -> &str,
    want: &str,
    extra: impl Fn(&T) -> bool,
) -> Option<&'a T> {
    let want = normalize(want);
    let mut candidates = items
        .iter()
        .filter(|i| extra(i) && normalize(name(i)).contains(&want));
    let first = candidates.next()?;
    if normalize(name(first)) == want {
        return Some(first);
    }
    candidates
        .find(|i| normalize(name(i)) == want)
        .or(Some(first))
}

fn matches(haystack: &str, needle: &str) -> bool {
    normalize(haystack).contains(&normalize(needle))
}

/// Lowercases and collapses punctuation to single spaces, so `AC/DC` matches `ac dc`.
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn keyword(word: &str) -> Option<Field> {
    match word.to_lowercase().as_str() {
        "artist" => Some(Field::Artist),
        "album" => Some(Field::Album),
        "track" | "song" => Some(Field::Track),
        _ => None,
    }
}

/// Recognizes `artist`, `artist:` and `artist:name`, returning the field and any trailing text.
fn split_keyword(word: &str) -> Option<(Field, &str)> {
    let (head, rest) = match word.split_once(':') {
        Some((head, rest)) => (head, rest),
        None => (word, ""),
    };
    keyword(head).map(|f| (f, rest))
}

struct Token {
    text: String,
    quoted: bool,
}

fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let text: String = chars.by_ref().take_while(|&c| c != '"').collect();
            tokens.push(Token { text, quoted: true });
        } else {
            let mut text = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '"' {
                    break;
                }
                text.push(c);
                chars.next();
            }
            tokens.push(Token {
                text,
                quoted: false,
            });
        }
    }
    tokens
}

/// Splits raw input into `(text, is_keyword)` runs for syntax highlighting, keeping every
/// character (including whitespace) so the rendered text lines up with the cursor.
pub fn highlight(input: &str) -> Vec<(&str, bool)> {
    let mut runs = Vec::new();
    let mut in_quote = false;
    let mut start = 0;
    let bytes: Vec<(usize, char)> = input.char_indices().collect();
    let mut i = 0;
    while i < bytes.len() {
        let (pos, c) = bytes[i];
        if c.is_whitespace() || in_quote || c == '"' {
            if c == '"' {
                in_quote = !in_quote;
            }
            i += 1;
            continue;
        }
        // Start of an unquoted word.
        let word_end = bytes[i..]
            .iter()
            .find(|(_, c)| c.is_whitespace() || *c == '"')
            .map_or(input.len(), |(p, _)| *p);
        let word = &input[pos..word_end];
        let head_len = word.find(':').map_or(word.len(), |p| p + 1);
        let head = word[..head_len].trim_end_matches(':');
        if keyword(head).is_some() {
            if start < pos {
                runs.push((&input[start..pos], false));
            }
            runs.push((&input[pos..pos + head_len], true));
            start = pos + head_len;
        }
        i += word.chars().count();
    }
    if start < input.len() {
        runs.push((&input[start..], false));
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(free: &str, artist: Option<&str>, album: Option<&str>, track: Option<&str>) -> Query {
        Query {
            free: free.into(),
            artist: artist.map(Into::into),
            album: album.map(Into::into),
            track: track.map(Into::into),
        }
    }

    #[test]
    fn plain_text() {
        assert_eq!(
            Query::parse("paranoid android"),
            q("paranoid android", None, None, None)
        );
    }

    #[test]
    fn single_fields() {
        assert_eq!(
            Query::parse("album ok computer"),
            q("", None, Some("ok computer"), None)
        );
        assert_eq!(
            Query::parse("Artist Radiohead"),
            q("", Some("Radiohead"), None, None)
        );
        assert_eq!(Query::parse("song creep"), q("", None, None, Some("creep")));
    }

    #[test]
    fn combined_fields_any_order() {
        let expected = q("", Some("radiohead"), Some("ok computer"), Some("airbag"));
        assert_eq!(
            Query::parse("artist radiohead album ok computer track airbag"),
            expected
        );
        assert_eq!(
            Query::parse("track airbag album ok computer artist radiohead"),
            expected
        );
    }

    #[test]
    fn colon_form_and_quotes() {
        assert_eq!(
            Query::parse("artist:ac/dc album:\"back in black\""),
            q("", Some("ac/dc"), Some("back in black"), None)
        );
        assert_eq!(
            Query::parse("album \"the artist\""),
            q("", None, Some("the artist"), None)
        );
    }

    #[test]
    fn bare_keyword_is_plain_text() {
        assert_eq!(Query::parse("album"), q("album", None, None, None));
    }

    #[test]
    fn free_text_before_keyword() {
        assert_eq!(
            Query::parse("live artist nirvana"),
            q("live", Some("nirvana"), None, None)
        );
    }

    #[test]
    fn normalization() {
        assert!(matches("AC/DC", "ac dc"));
        assert!(matches("OK Computer OKNOTOK 1997 2017", "ok computer"));
        assert!(!matches("Kid A", "ok computer"));
    }

    #[test]
    fn best_match_prefers_exact() {
        let names = ["OK Computer OKNOTOK", "OK Computer", "Kid A"];
        let got = best_match(&names, |s| s, "ok computer", |_| true);
        assert_eq!(got, Some(&"OK Computer"));
        assert_eq!(
            best_match(&names, |s| s, "oknotok", |_| true),
            Some(&"OK Computer OKNOTOK")
        );
    }

    #[test]
    fn highlight_keeps_all_text() {
        let input = "artist  radiohead album:\"the artist\" x";
        let runs = highlight(input);
        assert_eq!(runs.iter().map(|(s, _)| *s).collect::<String>(), input);
        let keywords: Vec<_> = runs.iter().filter(|(_, k)| *k).map(|(s, _)| *s).collect();
        assert_eq!(keywords, ["artist", "album:"]);
    }
}
