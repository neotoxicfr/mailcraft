//! Spell checking while writing: French and English Hunspell dictionaries (LibreOffice's,
//! bundled), checked with spellbook. A word is accepted when either language knows it, so mixed
//! French/English mail is not covered in red.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;

use spellbook::Dictionary;

struct Dicts {
    fr: Dictionary,
    en: Dictionary,
}

static DICTS: OnceLock<Option<Dicts>> = OnceLock::new();

fn load() -> Option<Dicts> {
    let fr = Dictionary::new(
        include_str!("../../../assets/dictionaries/fr.aff"),
        include_str!("../../../assets/dictionaries/fr.dic"),
    );
    let en = Dictionary::new(
        include_str!("../../../assets/dictionaries/en_US.aff"),
        include_str!("../../../assets/dictionaries/en_US.dic"),
    );
    match (fr, en) {
        (Ok(fr), Ok(en)) => Some(Dicts { fr, en }),
        (fr, en) => {
            log::warn!("spell checking off: {:?} {:?}", fr.err(), en.err());
            None
        }
    }
}

/// Loads the dictionaries on a background thread (a fraction of a second), so the first
/// message written does not wait for them.
pub fn warm_up() {
    if DICTS.get().is_none() {
        let _ = std::thread::Builder::new()
            .name("mailcraft-spell".into())
            .spawn(|| {
                DICTS.get_or_init(load);
            });
    }
}

/// The dictionaries are loaded (or failed to load: nothing more to wait for).
pub fn ready() -> bool {
    DICTS.get().is_some()
}

/// The dictionaries, once loaded; `None` meanwhile (no underlining yet).
fn dicts() -> Option<&'static Dicts> {
    DICTS.get().and_then(Option::as_ref)
}

/// Loads them now (tests).
#[cfg(test)]
fn dicts_now() -> &'static Dicts {
    DICTS.get_or_init(load).as_ref().expect("dictionaries")
}

fn known(d: &Dicts, word: &str, own: &HashSet<String>) -> bool {
    if own.contains(&word.to_lowercase()) || d.fr.check(word) || d.en.check(word) {
        return true;
    }
    // French elision: l'avion, qu'il, aujourd'hui is in the dictionary as a whole.
    for apostrophe in ['\'', '’'] {
        if let Some((head, tail)) = word.split_once(apostrophe) {
            let head = head.to_lowercase();
            let elided = [
                "l", "d", "j", "m", "n", "s", "t", "c", "qu", "jusqu", "lorsqu", "puisqu", "quoiqu",
            ];
            if elided.contains(&head.as_str()) && !tail.is_empty() {
                return known(d, tail, own);
            }
        }
    }
    // Hyphenated words the dictionary lacks as a whole: each part.
    if word.contains('-') {
        return word
            .split('-')
            .filter(|p| !p.is_empty())
            .all(|p| known(d, p, own));
    }
    false
}

fn skip(word: &str) -> bool {
    let letters = word.chars().filter(|c| c.is_alphabetic()).count();
    letters < 2
        || word.chars().any(|c| c.is_ascii_digit())
        // Acronyms (SNCF, PDF): left alone.
        || word.chars().all(|c| !c.is_lowercase())
}

/// Byte ranges of the unknown words of `text`. Addresses and links are skipped.
pub fn misspelled(text: &str, own: &HashSet<String>) -> Vec<Range<usize>> {
    let Some(d) = dicts() else {
        return Vec::new();
    };
    misspelled_with(d, text, own)
}

fn misspelled_with(d: &Dicts, text: &str, own: &HashSet<String>) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut chunk_start = 0;
    for chunk in text.split_inclusive(char::is_whitespace) {
        let start = chunk_start;
        chunk_start += chunk.len();
        if chunk.contains('@') || chunk.contains("://") || chunk.starts_with("www.") {
            continue;
        }
        let mut word: Option<usize> = None;
        let flush = |from: usize, to: usize, out: &mut Vec<Range<usize>>| {
            let w = chunk[from..to].trim_matches(['\'', '’', '-']);
            if w.is_empty() || skip(w) {
                return;
            }
            let lead =
                chunk[from..to].len() - chunk[from..to].trim_start_matches(['\'', '’', '-']).len();
            if !known(d, w, own) {
                out.push(start + from + lead..start + from + lead + w.len());
            }
        };
        for (i, ch) in chunk.char_indices() {
            let part = ch.is_alphanumeric() || ch == '\'' || ch == '’' || ch == '-';
            match (part, word) {
                (true, None) => word = Some(i),
                (false, Some(w)) => {
                    flush(w, i, &mut out);
                    word = None;
                }
                _ => {}
            }
        }
        if let Some(w) = word {
            flush(w, chunk.len(), &mut out);
        }
    }
    out
}

/// Up to 5 corrections, French first.
pub fn suggestions(word: &str) -> Vec<String> {
    let Some(d) = dicts() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    d.fr.suggest(word, &mut out);
    let mut en = Vec::new();
    d.en.suggest(word, &mut en);
    for w in en {
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out.truncate(5);
    out
}

/// The word around byte `at` in `text`, as a byte range.
pub fn word_at(text: &str, at: usize) -> Option<Range<usize>> {
    let is_part = |c: char| c.is_alphanumeric() || c == '\'' || c == '’' || c == '-';
    let at = at.min(text.len());
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_part(*c))
        .last()
        .map_or(at, |(i, _)| i);
    let end = text[at..]
        .char_indices()
        .find(|(_, c)| !is_part(*c))
        .map_or(text.len(), |(i, _)| at + i);
    let range = start..end;
    let w = text[range.clone()].trim_matches(['\'', '’', '-']);
    if w.is_empty() {
        return None;
    }
    let lead = text[range.clone()].len()
        - text[range.clone()]
            .trim_start_matches(['\'', '’', '-'])
            .len();
    Some(start + lead..start + lead + w.len())
}

#[cfg(test)]
mod tests {
    use super::{dicts_now, misspelled_with, word_at};
    use std::collections::HashSet;

    fn bad(text: &str) -> Vec<&str> {
        let own = HashSet::from(["mailcraft".to_owned()]);
        misspelled_with(dicts_now(), text, &own)
            .into_iter()
            .map(|r| &text[r])
            .collect()
    }

    #[test]
    fn french_and_english_words() {
        assert!(bad("Bonjour, je t'envoie l'avion aujourd'hui. Peut-être demain ?").is_empty());
        assert!(bad("Hello, see you tomorrow at the meeting.").is_empty());
        assert_eq!(bad("Bonjour, je vous envoie la facturre."), ["facturre"]);
        assert_eq!(bad("Merci pour le documment, à bientôt"), ["documment"]);
    }

    #[test]
    fn names_links_and_acronyms_are_left_alone() {
        assert!(bad("Écrire à alex@example.org ou voir https://exemple.fr/pagge").is_empty());
        assert!(bad("La SNCF et MailCraft, 12h30, PDF").is_empty());
    }

    #[test]
    fn word_under_the_caret() {
        let t = "voir l'avoin demain";
        let r = word_at(t, 8).unwrap();
        assert_eq!(&t[r], "l'avoin");
        assert!(word_at("  ", 1).is_none());
    }
}
