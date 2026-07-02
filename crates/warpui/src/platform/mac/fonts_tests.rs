use super::*;

// Loads a macOS system font family through the same inherent APIs the app uses.
fn load_family(db: &mut FontDB, name: &str) -> FamilyId {
    let family = loader::load_system_font(name).expect("system font family should load");
    db.insert_font_family(family).expect("family should insert")
}

// Regression test for #12923: within a single PostScript-name bucket, identity must be
// resolved by CGFont equality, so two genuinely different fonts sharing a name (a
// user/system-installed font vs. a bundled one) each resolve to their OWN font_id. Name-based
// resolution conflated them and returned the wrong font's outlines, misrendering shaped glyphs
// (e.g. `fi`->`ç`, `•`->`ǎ`).
#[test]
fn font_id_by_identity_disambiguates_same_named_fonts() {
    let mut db = FontDB::new();
    let helvetica = load_family(&mut db, "Helvetica");
    let menlo = load_family(&mut db, "Menlo");

    let id_helvetica = db.select_font(helvetica, Properties::default());
    let id_menlo = db.select_font(menlo, Properties::default());
    assert_ne!(id_helvetica, id_menlo);

    let nf_helvetica = db.native_font(id_helvetica, DEFAULT_FONT_SIZE as f32);
    let nf_menlo = db.native_font(id_menlo, DEFAULT_FONT_SIZE as f32);

    // Bucket both distinct fonts under one shared PostScript name, exactly as happens when a
    // user font shares a bundled font's name.
    let shared = "SharedPostScriptName";
    db.fonts_by_name.insert(
        Arc::new(shared.to_string()),
        vec![
            (nf_helvetica.clone(), id_helvetica),
            (nf_menlo.clone(), id_menlo),
        ],
    );

    // Each font must resolve to its own id despite sharing the bucket.
    assert_eq!(
        db.font_id_by_identity(shared, &nf_helvetica),
        Some(id_helvetica)
    );
    assert_eq!(db.font_id_by_identity(shared, &nf_menlo), Some(id_menlo));
}

// The identity round-trip must hold across font sizes: a run comes back at the shaping size,
// while registered entries are stored at DEFAULT_FONT_SIZE, yet both share one CGFont, so the
// lookup must still resolve to the originally-selected font_id.
#[test]
fn font_id_for_native_font_round_trips_across_sizes() {
    let mut db = FontDB::new();
    let helvetica = load_family(&mut db, "Helvetica");
    let id = db.select_font(helvetica, Properties::default());

    for size in [11.0_f32, 24.0, 48.0] {
        let nf = db.native_font(id, size);
        assert_eq!(
            db.font_id_for_native_font(nf),
            id,
            "font at size {size} should resolve to its selected id"
        );
    }
}

// The identity comparator must treat the same font as equal and different fonts as distinct.
#[test]
fn same_native_font_distinguishes_different_fonts() {
    let mut db = FontDB::new();
    let helvetica = load_family(&mut db, "Helvetica");
    let menlo = load_family(&mut db, "Menlo");

    let id_h = db.select_font(helvetica, Properties::default());
    let id_m = db.select_font(menlo, Properties::default());
    let nf_helvetica = db.native_font(id_h, DEFAULT_FONT_SIZE as f32);
    let nf_menlo = db.native_font(id_m, DEFAULT_FONT_SIZE as f32);

    assert!(same_native_font(&nf_helvetica, &nf_helvetica));
    assert!(!same_native_font(&nf_helvetica, &nf_menlo));
}
