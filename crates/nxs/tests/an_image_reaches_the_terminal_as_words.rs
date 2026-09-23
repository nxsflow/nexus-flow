//! A guide page is TWO surfaces from one file: the website renders it, and `nxs guide` prints it.
//! An image is the one inline form where those two cannot agree — the terminal has no picture to
//! draw — and before 6j6v.jepw the renderer had no case for it at all, so `![alt](url)` came out
//! as `!alt`: neither the picture nor a sentence.
//!
//! What this file holds is the MECHANISM, deliberately not the prose. A whole-page golden (the
//! shape `guide.trycmd` uses for `getting-started`) would re-stamp on every wording edit while
//! answering this question only incidentally. These assertions stay true across any rewrite of the
//! page and go red the moment the image case stops working.

use assert_cmd::Command;

fn guide_page(topic: &str) -> String {
    let out = Command::cargo_bin("nxs")
        .expect("nxs")
        .env("NXC_TIMER", "dry")
        .args(["guide", topic])
        .output()
        .expect("binary runs");
    assert!(
        out.status.success(),
        "`nxs guide {topic}` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

#[test]
fn the_architecture_page_renders_its_diagram_as_words_not_markdown() {
    let page = guide_page("architecture");

    // The alt text arrives as a marked-up sentence a reader can act on…
    assert!(
        page.contains("[image: "),
        "the image line did not render as a marker:\n{page}"
    );
    // …and NOT as the raw markdown that leaked before this case existed.
    assert!(
        !page.contains("!["),
        "raw markdown image syntax reached the terminal:\n{page}"
    );
    // The image TARGET is a build artifact, not something a terminal reader can open, so the
    // marker must drop it. Anchored on the asset URL and NOT on the file extension: a page is
    // free to name an image format in its own prose, and an earlier version of this assertion —
    // which banned the extension outright — went red on exactly such a sentence. The url is the
    // thing that has no business in terminal output; the extension is just a word.
    assert!(
        !page.contains("/nxs/docs/assets/"),
        "the asset path leaked into terminal output:\n{page}"
    );
}

#[test]
fn the_architecture_page_carries_the_topology_without_the_picture() {
    let page = guide_page("architecture");

    // The DoD for 6j6v.jepw names what the depiction must show. The picture is an illustration;
    // the TEXT is what an agent reading this in a terminal actually gets, so the text is what is
    // asserted here. Each probe is a distinct layer, so a page that quietly loses one goes red.
    for required in [
        "argv[0]",        // the four personas over one binary
        "crates/facade",  // the SemVer-gated engine seam
        ".nxs/db.sqlite", // the one shared workspace store
        "append-only",    // the op-log as the single source of truth
        "nxf-relay",      // the outward sync edge
    ] {
        assert!(
            page.contains(required),
            "the architecture page never mentions `{required}`:\n{page}"
        );
    }

    // The one-way substrate arrow, which the picture draws and the text must state.
    assert!(
        page.contains("materialized views"),
        "the ops → fold → views → derivation direction is missing:\n{page}"
    );
}
