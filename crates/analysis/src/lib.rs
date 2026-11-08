//! What MailCraft can tell about a sender without any AI and without sending anything anywhere:
//! how to unsubscribe (RFC 2369 `List-Unsubscribe`, RFC 8058 one-click), whether the mail passed
//! SPF, DKIM and DMARC (`Authentication-Results`, RFC 8601), and signs of impersonation (a
//! brand in the display name on another domain, lookalike or punycode domains).

/// How a sender can be left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unsubscribe {
    /// RFC 8058: a POST to `https` with `List-Unsubscribe=One-Click` is enough.
    pub one_click: Option<String>,
    pub mailto: Option<String>,
    /// Only a web page: opened for the user, never clicked by MailCraft.
    pub web: Option<String>,
}

impl Unsubscribe {
    pub fn is_possible(&self) -> bool {
        self.one_click.is_some() || self.mailto.is_some() || self.web.is_some()
    }
}

/// Parses `List-Unsubscribe` (`<mailto:…>, <https://…>`) and `List-Unsubscribe-Post`.
pub fn unsubscribe(
    list_unsubscribe: Option<&str>,
    list_unsubscribe_post: Option<&str>,
) -> Unsubscribe {
    let mut out = Unsubscribe::default();
    let Some(header) = list_unsubscribe else {
        return out;
    };
    let one_click = list_unsubscribe_post.is_some_and(|p| {
        p.split(';')
            .any(|kv| kv.trim().eq_ignore_ascii_case("List-Unsubscribe=One-Click"))
    });
    for part in header.split(',') {
        let uri = part
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim();
        let lower = uri.to_ascii_lowercase();
        if lower.starts_with("mailto:") && out.mailto.is_none() {
            out.mailto = Some(uri.to_owned());
        } else if lower.starts_with("https://") && out.web.is_none() {
            out.web = Some(uri.to_owned());
        }
    }
    // One-click needs https (RFC 8058 §3.1); the web link then is the one-click target.
    if one_click {
        out.one_click = out.web.take();
    }
    out
}

/// Destination and subject of a `mailto:` unsubscribe link.
pub fn mailto_parts(mailto: &str) -> Option<(String, String)> {
    let rest = mailto.get(7..)?;
    let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
    let addr = url::form_urlencoded::parse(addr.as_bytes())
        .map(|(k, _)| k.into_owned())
        .next()
        .unwrap_or_default();
    if !addr.contains('@') {
        return None;
    }
    let subject = url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k.eq_ignore_ascii_case("subject"))
        .map_or_else(|| "unsubscribe".to_owned(), |(_, v)| v.into_owned());
    Some((addr, subject))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Check {
    Pass,
    Fail,
    /// softfail, neutral, temperror… or the header says nothing.
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Auth {
    pub spf: Check,
    pub dkim: Check,
    pub dmarc: Check,
}

/// Reads `spf=`, `dkim=`, `dmarc=` from `Authentication-Results` headers (several may be given).
pub fn authentication(headers: &[&str]) -> Auth {
    let mut auth = Auth::default();
    for h in headers {
        let lower = h.to_ascii_lowercase();
        for (key, slot) in [
            ("spf=", &mut auth.spf),
            ("dkim=", &mut auth.dkim),
            ("dmarc=", &mut auth.dmarc),
        ] {
            // Any pass wins (several DKIM signatures); a fail only if nothing passed.
            for (at, _) in lower.match_indices(key) {
                // "compauth=" and similar contain no such key; make sure it starts a token.
                if at > 0 && lower.as_bytes()[at - 1].is_ascii_alphanumeric() {
                    continue;
                }
                let value: String = lower[at + key.len()..]
                    .chars()
                    .take_while(char::is_ascii_alphabetic)
                    .collect();
                match value.as_str() {
                    "pass" => *slot = Check::Pass,
                    "fail" | "permfail" if *slot != Check::Pass => *slot = Check::Fail,
                    _ => {}
                }
            }
        }
    }
    auth
}

/// Brands phishing mail likes to pretend to be, with the domains they really send from.
const BRANDS: &[(&str, &[&str])] = &[
    ("paypal", &["paypal.com", "paypal.fr"]),
    ("amazon", &["amazon.com", "amazon.fr", "amazonses.com"]),
    (
        "microsoft",
        &[
            "microsoft.com",
            "outlook.com",
            "live.com",
            "office.com",
            "office365.com",
        ],
    ),
    ("outlook", &["microsoft.com", "outlook.com", "live.com"]),
    ("apple", &["apple.com", "icloud.com"]),
    ("icloud", &["apple.com", "icloud.com"]),
    ("google", &["google.com", "gmail.com", "youtube.com"]),
    ("netflix", &["netflix.com"]),
    (
        "facebook",
        &["facebook.com", "facebookmail.com", "meta.com"],
    ),
    (
        "instagram",
        &["instagram.com", "facebookmail.com", "mail.instagram.com"],
    ),
    ("whatsapp", &["whatsapp.com"]),
    ("dropbox", &["dropbox.com"]),
    ("docusign", &["docusign.com", "docusign.net"]),
    ("ameli", &["ameli.fr", "assurance-maladie.fr"]),
    ("impots", &["impots.gouv.fr", "dgfip.finances.gouv.fr"]),
    ("caf", &["caf.fr"]),
    ("antai", &["antai.gouv.fr"]),
    (
        "la poste",
        &["laposte.fr", "laposte.net", "notification.laposte.fr"],
    ),
    ("colissimo", &["laposte.fr", "colissimo.fr"]),
    ("chronopost", &["chronopost.fr"]),
    ("dhl", &["dhl.com", "dhl.fr", "dhl.de"]),
    ("ups", &["ups.com"]),
    ("fedex", &["fedex.com"]),
    ("edf", &["edf.fr"]),
    ("engie", &["engie.fr", "engie.com"]),
    ("orange", &["orange.fr", "orange.com"]),
    ("sfr", &["sfr.fr"]),
    ("bouygues", &["bouyguestelecom.fr"]),
    // The ISP, not the English word "free" (Free spins, free shipping…).
    ("freebox", &["free.fr", "freetelecom.fr", "free-mobile.fr"]),
    (
        "free mobile",
        &["free.fr", "freetelecom.fr", "free-mobile.fr"],
    ),
    ("credit agricole", &["credit-agricole.fr", "ca-paris.fr"]),
    (
        "bnp",
        &["bnpparibas", "bnpparibas.com", "mabanque.bnpparibas"],
    ),
    ("societe generale", &["societegenerale.fr", "socgen.com"]),
    ("banque postale", &["labanquepostale.fr"]),
    ("lcl", &["lcl.fr"]),
    ("boursorama", &["boursorama.fr", "boursorama.com"]),
    ("crédit mutuel", &["creditmutuel.fr"]),
    ("credit mutuel", &["creditmutuel.fr"]),
    ("visa", &["visa.com", "visa.fr"]),
    ("mastercard", &["mastercard.com"]),
    ("binance", &["binance.com"]),
    ("coinbase", &["coinbase.com"]),
];

/// The registrable part of a domain, roughly: the last two labels (three for `gouv.fr`, `co.uk`…).
pub fn base_domain(domain: &str) -> String {
    let d = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = d.split('.').collect();
    let n = labels.len();
    if n <= 2 {
        return d;
    }
    let second = labels[n - 2];
    let take = if matches!(second, "co" | "com" | "gouv" | "org" | "net" | "ac" | "gov")
        && labels[n - 1].len() == 2
    {
        3
    } else {
        2
    };
    labels[n.saturating_sub(take)..].join(".")
}

pub fn domain_of(address: &str) -> String {
    address
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'à' | 'â' | 'ä' => 'a',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// Reasons to distrust a sender, most serious first. Empty when nothing looks wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Warning {
    /// Display name claims a brand the address does not belong to.
    Impersonation { brand: String },
    /// Domain imitates a brand domain (`paypa1.com`, `micros0ft-support.com`).
    Lookalike { brand: String },
    /// Internationalised domain (`xn--`): letters from other alphabets can look identical.
    Punycode,
    /// DMARC failed: the From domain did not authorise this mail.
    DmarcFail,
    /// SPF and DKIM both failed.
    AuthFail,
    /// The mail claims to come from the user's own address (a classic scam).
    OwnAddress,
    /// Not a real domain: no dot, labels starting or ending with "-", a top-level part that
    /// is not letters.
    InvalidDomain,
    /// Machine-generated look: capitals or long runs of random letters and digits.
    RandomDomain,
    /// The display name is a domain other than the sender's ("MaBoutique.fr" from another domain).
    DisplayDomain { shown: String },
}

/// `own` lists the user's addresses (a mail "from yourself" that is not is a scam).
pub fn warnings(display_name: &str, address: &str, auth: Auth, own: &[String]) -> Vec<Warning> {
    let mut out = Vec::new();
    let domain = domain_of(address);
    let base = base_domain(&domain);
    let name = fold(display_name);
    let owned = |domains: &[&str]| {
        domains
            .iter()
            .any(|d| domain == *d || domain.ends_with(&format!(".{d}")) || base == *d)
    };
    for (brand, domains) in BRANDS {
        let word = |hay: &str| {
            hay.split(|c: char| !c.is_alphanumeric() && c != ' ')
                .any(|part| {
                    part == *brand
                        || part
                            .split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .contains(brand)
                })
        };
        // Short brands ("caf", "ups", "lcl", "sfr", "free", "visa") only as whole words.
        let in_name = if brand.len() <= 4 {
            name.split(|c: char| !c.is_alphanumeric())
                .any(|w| w == *brand)
        } else {
            word(&name)
        };
        if in_name && !owned(domains) {
            out.push(Warning::Impersonation {
                brand: (*brand).to_owned(),
            });
            break;
        }
    }
    let label = base.split('.').next().unwrap_or("");
    for (brand, domains) in BRANDS {
        let b = brand.replace(' ', "");
        if b.len() < 5 || owned(domains) {
            continue;
        }
        let unconfused: String = label
            .chars()
            .map(|c| match c {
                '0' => 'o',
                '1' | 'l' => 'l',
                '3' => 'e',
                '5' => 's',
                '7' => 't',
                c => c,
            })
            .collect();
        let target: String = b.chars().map(|c| if c == 'i' { 'l' } else { c }).collect();
        let near = unconfused.replace('i', "l").contains(&target) || distance(label, &b) == 1;
        if near && !matches!(out.first(), Some(Warning::Impersonation { brand: x }) if *x == *brand)
        {
            out.push(Warning::Lookalike {
                brand: (*brand).to_owned(),
            });
            break;
        }
    }
    if domain.split('.').any(|l| l.starts_with("xn--")) {
        out.push(Warning::Punycode);
    }
    if own.iter().any(|o| o.eq_ignore_ascii_case(address.trim())) && auth.dmarc != Check::Pass {
        out.push(Warning::OwnAddress);
    }
    // The raw address keeps its case: generators use capitals, real senders almost never do.
    let raw_domain = address.rsplit_once('@').map_or("", |(_, d)| d.trim());
    if invalid_domain(&domain) {
        out.push(Warning::InvalidDomain);
    } else if random_looking(raw_domain) {
        out.push(Warning::RandomDomain);
    }
    if let Some(shown) = shown_domain(display_name)
        && base_domain(&shown) != base
        && !out
            .iter()
            .any(|w| matches!(w, Warning::Impersonation { .. }))
    {
        out.push(Warning::DisplayDomain { shown });
    }
    if auth.dmarc == Check::Fail {
        out.push(Warning::DmarcFail);
    } else if auth.spf == Check::Fail && auth.dkim == Check::Fail {
        out.push(Warning::AuthFail);
    }
    out
}

fn invalid_domain(domain: &str) -> bool {
    let labels: Vec<&str> = domain.split('.').collect();
    let tld = labels.last().copied().unwrap_or("");
    labels.len() < 2
        || labels
            .iter()
            .any(|l| l.is_empty() || l.starts_with('-') || l.ends_with('-'))
        || tld.len() < 2
        || !tld.chars().all(|c| c.is_ascii_alphabetic()) && !tld.starts_with("xn--")
}

/// Capital letters, or a label of 10+ characters mixing digits and letters, or 6+ consonants in a row.
fn random_looking(raw_domain: &str) -> bool {
    if raw_domain.chars().any(|c| c.is_ascii_uppercase()) {
        return true;
    }
    raw_domain
        .split('.')
        .rev()
        .skip(1)
        .flat_map(|l| l.split('-'))
        .any(|label| {
            // Digits and letters switching back and forth ("id1nxs1a9hdl…"), not "office365" or "b2b".
            let chars: Vec<char> = label.chars().collect();
            let switches = chars
                .windows(2)
                .filter(|w| w[0].is_ascii_digit() != w[1].is_ascii_digit())
                .count();
            let mixed = label.len() >= 8 && switches >= 3;
            let mut run = 0;
            let mut longest = 0;
            for c in label.chars() {
                if c.is_ascii_alphabetic() && !"aeiouy".contains(c) {
                    run += 1;
                    longest = longest.max(run);
                } else {
                    run = 0;
                }
            }
            mixed || longest >= 6
        })
}

/// A domain written as the display name ("MaBoutique.fr", "service@paypal.com").
fn shown_domain(display_name: &str) -> Option<String> {
    let name = display_name.trim().to_ascii_lowercase();
    let candidate = name.rsplit_once('@').map_or(name.as_str(), |(_, d)| d);
    let candidate = candidate.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let ok = !candidate.contains(' ')
        && candidate.contains('.')
        && candidate
            .rsplit('.')
            .next()
            .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
    ok.then(|| candidate.to_owned())
}

/// Levenshtein distance (small strings only).
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push(
                (prev[j] + usize::from(ca != cb))
                    .min(prev[j + 1] + 1)
                    .min(cur[j] + 1),
            );
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Local guess of the kind of sender, before (or without) Claude.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Suspect,
    /// Mailing list or marketing: has an unsubscribe header.
    Newsletter,
    Other,
}

pub fn category(unsubscribe: &Unsubscribe, warnings: &[Warning]) -> Category {
    if !warnings.is_empty() {
        Category::Suspect
    } else if unsubscribe.is_possible() {
        Category::Newsletter
    } else {
        Category::Other
    }
}

#[cfg(test)]
mod tests;
