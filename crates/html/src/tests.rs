use super::{Block, Kind, from_text, parse, parse_with, to_text, tracker_urls, without_trackers};

fn texts(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .map(|b| b.inlines.iter().map(|i| i.text.as_str()).collect())
        .collect()
}

#[test]
fn structure_and_emphasis() {
    let b = parse(
        "<h1>Bonjour</h1><p>Votre <b>facture</b> de <a href='https://example.com/f'>novembre</a>\n  est   prête.</p>\
         <ul><li>un</li><li>deux<ol><li>a</li></ol></li></ul><blockquote><p>cité</p></blockquote><hr><pre>a  b\nc</pre>",
    );
    assert_eq!(
        texts(&b),
        [
            "Bonjour",
            "Votre facture de novembre est prête.",
            "un",
            "deux",
            "a",
            "cité",
            "",
            "a  b\nc"
        ]
    );
    assert_eq!(b[0].kind, Kind::Heading(1));
    let p = &b[1].inlines;
    assert!(p[1].bold && p[1].text == "facture");
    assert_eq!(p[3].link.as_deref(), Some("https://example.com/f"));
    assert_eq!(
        b[4].kind,
        Kind::Item {
            ordered: true,
            number: 1,
            depth: 1
        }
    );
    assert_eq!(b[5].quote, 1);
    assert_eq!(b[6].kind, Kind::Rule);
    assert_eq!(b[7].kind, Kind::Pre);
}

#[test]
fn hostile_content_is_dropped() {
    let b = parse(
        "<script>alert(1)</script><style>p{color:red}</style>\
         <p><a href='javascript:alert(1)'>clic</a> <a href=' data:text/html,x'>data</a> <a href='mailto:a@example.com'>m</a></p>\
         <div style='display: none'>texte caché</div><span hidden>caché</span>\
         <img src='https://track.example.com/p.gif' width='1' height='1'>\
         <img src='https://example.com/logo.png' alt='Logo'><img src='file:///C:/x.png'><img src='cid:img1@x'>\
         <form><input value='x'><button>Envoyer</button></form>",
    );
    let all = to_text(&b);
    assert!(
        !all.contains("alert")
            && !all.contains("color")
            && !all.contains("caché")
            && !all.contains("Envoyer")
    );
    let links: Vec<_> = b
        .iter()
        .flat_map(|b| &b.inlines)
        .filter_map(|i| i.link.as_deref())
        .collect();
    assert_eq!(links, ["mailto:a@example.com"]);
    let images: Vec<_> = b
        .iter()
        .filter_map(|b| match &b.kind {
            Kind::Image { src, .. } => Some(src.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(images, ["https://example.com/logo.png", "cid:img1@x"]);
}

#[test]
fn layout_tables_flow_and_unclosed_tags_survive() {
    let b = parse("<table><tr><td>Total</td><td><b>42 €</td></tr><tr><td>Merci");
    assert_eq!(texts(&b), ["Total 42 €", "Merci"]);
}

#[test]
fn huge_and_deep_input_is_bounded_and_fast() {
    let big = "<table><tr><td><p>ligne</p></td></tr></table>".repeat(40_000);
    let start = std::time::Instant::now();
    let b = parse(&big);
    assert!(b.len() <= super::MAX_BLOCKS);
    let deep = format!(
        "{}texte{}",
        "<div><b>".repeat(5_000),
        "</b></div>".repeat(5_000)
    );
    let d = parse(&deep);
    assert_eq!(texts(&d), ["texte"]);
    let ms = start.elapsed().as_millis();
    // Debug build; release is several times faster.
    assert!(ms < 5_000, "{ms} ms for {} bytes", big.len() + deep.len());
}

#[test]
fn plain_text_mail() {
    let b = from_text("Bonjour\n\n> cité\nFin");
    assert_eq!(texts(&b), ["Bonjour", "", "> cité", "Fin"]);
    assert_eq!(b[2].quote, 1);
}

/// Regression (real mail, MJML layout): containers with `font-size:0` hold all the text.
#[test]
fn zero_font_size_containers_keep_their_text() {
    let b = parse(
        "<div style=\"display:none; font-size:1px; max-height:0px; overflow:hidden\">Aperçu caché</div>\
         <div style=\"font-size:0px; text-align:left; display:inline-block; width:100%\">\
           <table><tr><td style=\"font-size:0px; padding:23px\">\
             <div style=\"font-size:16px\">Votre vol est confirmé</div></td></tr></table></div>",
    );
    assert_eq!(texts(&b), ["Votre vol est confirmé"]);
}

/// Regression (real phishing mail): content made only of CSS background images.
#[test]
fn background_images_become_images() {
    let b = parse(
        "<div align=\"center\"><a href=\"http://example.com/x\" style=\"display:block; width:652px; \
         background:url('https://cdn.example.com/a.png') no-repeat\"></a>\
         <table><tr><td background=\"https://cdn.example.com/b.jpg\">texte</td></tr></table>\
         <div style=\"background-image: url(javascript:alert(1))\">c</div></div>",
    );
    let images: Vec<_> = b
        .iter()
        .filter_map(|b| match &b.kind {
            Kind::Image { src, .. } => Some(src.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        images,
        [
            "https://cdn.example.com/a.png",
            "https://cdn.example.com/b.jpg"
        ]
    );
}

#[test]
fn tracking_pixels_are_recognised() {
    let html = r#"<p>Bonjour</p>
<img src="https://cdn.shop.example/banner.jpg" width="600" alt="Soldes">
<img src="https://t.shop.example/o?id=1&amp;u=2" width="1" height="1">
<img src="https://u123.ct.sendgrid.net/wf/open?upn=abc">
<img src="https://x.example/p.gif" style="display: none">
<img src="https://x.example/q.gif" style="width:1px;height:1px">"#;
    let urls = tracker_urls(html);
    assert_eq!(urls.len(), 4, "{urls:?}");
    assert!(!urls.iter().any(|u| u.contains("banner")));
    let clean = without_trackers(html);
    assert!(clean.contains("banner.jpg"));
    assert!(
        !clean.contains("t.shop.example")
            && !clean.contains("sendgrid")
            && !clean.contains("p.gif")
    );
    // Simplified view: dropped by default, kept on demand.
    let images = |b: &[Block]| {
        b.iter()
            .filter(|b| matches!(b.kind, Kind::Image { .. }))
            .count()
    };
    assert_eq!(images(&parse(html)), 1);
    assert_eq!(images(&parse_with(html, false)), 4); // only the display:none one stays out
}
