use super::{
    Auth, Category, Check, Unsubscribe, Warning, authentication, base_domain, category,
    mailto_parts, unsubscribe, warnings,
};

#[test]
fn unsubscribe_headers() {
    let u = unsubscribe(
        Some("<mailto:unsub@example.com?subject=stop>, <https://example.com/u?id=1>"),
        Some("List-Unsubscribe=One-Click"),
    );
    assert_eq!(u.one_click.as_deref(), Some("https://example.com/u?id=1"));
    assert_eq!(
        u.mailto.as_deref(),
        Some("mailto:unsub@example.com?subject=stop")
    );
    assert!(u.web.is_none());

    // No Post header: the https link is only a page to open.
    let u = unsubscribe(Some("<https://example.com/u>"), None);
    assert_eq!(
        (u.one_click, u.web.as_deref()),
        (None, Some("https://example.com/u"))
    );
    // http (not https) is ignored; nothing else either.
    assert!(
        !unsubscribe(
            Some("<http://example.com/u>"),
            Some("List-Unsubscribe=One-Click")
        )
        .is_possible()
    );
    assert_eq!(unsubscribe(None, None), Unsubscribe::default());

    assert_eq!(
        mailto_parts("mailto:unsub@example.com?subject=stop%20please").unwrap(),
        ("unsub@example.com".into(), "stop please".into())
    );
    assert_eq!(
        mailto_parts("mailto:list@example.com").unwrap().1,
        "unsubscribe"
    );
    assert!(mailto_parts("mailto:?subject=x").is_none());
}

#[test]
fn authentication_results() {
    let a = authentication(&[
        "spf=pass (sender IP is 1.2.3.4) smtp.mailfrom=example.com; dkim=fail; dkim=pass header.d=example.com; \
         dmarc=pass action=none header.from=example.com;compauth=pass reason=100",
    ]);
    assert_eq!(
        a,
        Auth {
            spf: Check::Pass,
            dkim: Check::Pass,
            dmarc: Check::Pass
        }
    );
    let a = authentication(&["spf=softfail; dkim=none; dmarc=fail action=quarantine"]);
    assert_eq!(
        a,
        Auth {
            spf: Check::Unknown,
            dkim: Check::Unknown,
            dmarc: Check::Fail
        }
    );
    assert_eq!(authentication(&[]), Auth::default());
}

#[test]
fn base_domains() {
    assert_eq!(base_domain("mail.news.example.com"), "example.com");
    assert_eq!(base_domain("dgfip.finances.gouv.fr"), "finances.gouv.fr");
    assert_eq!(base_domain("shop.example.co.uk"), "example.co.uk");
    assert_eq!(base_domain("example.com."), "example.com");
}

#[test]
fn impersonation_and_lookalikes() {
    let ok = Auth {
        spf: Check::Pass,
        dkim: Check::Pass,
        dmarc: Check::Pass,
    };
    assert!(warnings("PayPal", "service@paypal.com", ok, &[]).is_empty());
    assert!(warnings("Amazon.fr", "commande@amazon.fr", ok, &[]).is_empty());
    assert!(warnings("Ameli", "noreply@info.ameli.fr", ok, &[]).is_empty());
    assert_eq!(
        warnings("PayPal Service", "x@random-shop.ru", ok, &[]),
        [Warning::Impersonation {
            brand: "paypal".into()
        }]
    );
    assert_eq!(
        warnings(
            "Service Impôts",
            "contact@finances-remboursement.com",
            ok,
            &[]
        ),
        [Warning::Impersonation {
            brand: "impots".into()
        }]
    );
    assert!(
        warnings("Paypa1", "security@paypa1.com", ok, &[]).contains(&Warning::Lookalike {
            brand: "paypal".into()
        })
    );
    assert!(
        warnings("Support", "help@micros0ft-support.com", ok, &[]).contains(&Warning::Lookalike {
            brand: "microsoft".into()
        })
    );
    assert!(warnings("Shop", "a@xn--pypal-4ve.com", ok, &[]).contains(&Warning::Punycode));
    // Short brands only as whole words: "Upsilon" is not UPS, "Cafe" is not CAF.
    assert!(warnings("Upsilon Cafe", "hello@upsilon.example", ok, &[]).is_empty());
    // Authentication.
    let dmarc_fail = Auth {
        dmarc: Check::Fail,
        ..ok
    };
    assert_eq!(
        warnings("Shop", "a@shop.example", dmarc_fail, &[]),
        [Warning::DmarcFail]
    );
    let both_fail = Auth {
        spf: Check::Fail,
        dkim: Check::Fail,
        dmarc: Check::Unknown,
    };
    assert_eq!(
        warnings("Shop", "a@shop.example", both_fail, &[]),
        [Warning::AuthFail]
    );
}

#[test]
fn local_category() {
    let news = unsubscribe(Some("<https://example.com/u>"), None);
    assert_eq!(category(&news, &[]), Category::Newsletter);
    assert_eq!(category(&news, &[Warning::Punycode]), Category::Suspect);
    assert_eq!(category(&Unsubscribe::default(), &[]), Category::Other);
}

/// Patterns seen in a real mailbox (anonymised): spam with generated domains, mail claiming to
/// come from the user's own address, a display name showing another domain.
#[test]
fn generated_domains_own_address_and_shown_domain() {
    let ok = Auth {
        spf: Check::Pass,
        dkim: Check::Pass,
        dmarc: Check::Pass,
    };
    let unknown = Auth::default();
    for spam in [
        "info@-----mail.JcJm27eVjK.com",
        "no-reply@buevnibgcjpykhaqcwxyjahmr",
        "no-reply@1zr1Qp3.🥹🫶",
        "X@ID1NXS1A9HDLNFQAFONVSNJ5FL56.com",
        "outlet13151@xkqrtzvnm.info",
    ] {
        let w = warnings("Promo", spam, unknown, &[]);
        assert!(
            w.contains(&Warning::InvalidDomain) || w.contains(&Warning::RandomDomain),
            "{spam}: {w:?}"
        );
    }
    for fine in [
        "news@newsletter.example.fr",
        "contact@mail.shop-online.example",
        "noreply@info.ameli.fr",
        "a@b2b-office365.example",
    ] {
        assert!(warnings("Shop", fine, ok, &[]).is_empty(), "{fine}");
    }
    let me = vec!["me@example.com".to_string()];
    assert!(warnings("Me", "ME@example.com", unknown, &me).contains(&Warning::OwnAddress));
    assert!(!warnings("Me", "me@example.com", ok, &me).contains(&Warning::OwnAddress));
    assert_eq!(
        warnings("MaBoutique.fr", "no-reply@shop.example", ok, &[]),
        [Warning::DisplayDomain {
            shown: "maboutique.fr".into()
        }]
    );
    assert!(warnings("shop.example", "no-reply@news.shop.example", ok, &[]).is_empty());
    // "Free spins" is not the ISP Free; "Freebox" is.
    assert!(warnings("Free spins", "promo@casino.example", ok, &[]).is_empty());
    assert_eq!(
        warnings("Freebox", "x@casino.example", ok, &[]),
        [Warning::Impersonation {
            brand: "freebox".into()
        }]
    );
}
