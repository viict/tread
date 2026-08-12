//! The seam's vocabulary: the handles, the state and the small records every
//! [`Source`](super::Source) hands back.
//!
//! None of it is a format and none of it has behaviour — split out of `mod.rs`
//! so that file is the trait and its contract, and nothing else.
#![deny(unsafe_code)]

/// A place in the document that survives folding but not re-layout.
///
/// Opaque to everything above the seam: the pager only ever compares anchors
/// (they order the same way the document reads) and hands them back to the
/// source it got them from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Anchor(pub usize);

/// A place in the *content* that survives re-layout, so a resize can put the
/// cursor back where it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mark(pub usize);

/// Opaque, format-defined fold state: the ids of the closed sections.
///
/// The pager stores it in a history [`Snapshot`](crate::nav::history::Snapshot)
/// and hands it back verbatim; it never inspects an id. Ids must be stable
/// across re-layout, which is what lets folds survive a resize.
pub type FoldState = Vec<String>;

/// One entry of the document outline (`o`, and the collapse tree).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Nesting depth, 1 = outermost. Drives indentation and the fold ranges.
    pub level: u8,
    /// Stable id for this section: the key fold state is stored under, and the
    /// target of an anchor link (`#some-heading`).
    pub id: String,
    /// Text shown in the outline overlay.
    pub text: String,
    /// Where the section starts.
    pub anchor: Anchor,
    /// True when this section is currently folded shut.
    pub folded: bool,
}

/// One link occurrence in the document, in reading order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSite {
    /// The row the link sits on, as an anchor (it may be folded away).
    pub anchor: Anchor,
    /// Display column the link starts at, within its row.
    pub col: usize,
    pub url: String,
}

/// One search match on a row, in display columns of that row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchSpan {
    pub start: usize,
    pub end: usize,
    /// True for the match the cursor is currently sitting on.
    pub current: bool,
}

/// Where `G` lands, and whether the format still has work to do to know.
///
/// A format that discovers its document lazily — a CSV's row index — genuinely
/// does not know where the end is until it has scanned there, and the *worst*
/// answer is the confident one: jumping to the end of whatever happens to be
/// indexed puts the cursor in the middle of the file and says nothing about it.
/// [`End::Scanning`] is that honest "not yet", carrying the percentage the
/// status bar shows while the pager drives the scan a slice at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The last row `G` should put the cursor on.
    At(usize),
    /// The end is not known yet; `0..=100` of the way there.
    Scanning(u8),
}

/// Where a search landed, and whether it wrapped around the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub anchor: Anchor,
    pub wrapped: bool,
}

/// One row expanded into labelled fields.
///
/// A grid shows as many columns as fit and no more, which is exactly wrong for
/// the row you actually care about: a wide CSV hides most of it off-screen, and
/// a ragged row can carry fields the header never named. This is that row read
/// the other way round — one field per line, label beside value, nothing
/// hidden. A future tree format would return a node's children the same way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detail {
    /// What the overlay is titled, e.g. `Row 41`.
    pub title: String,
    /// `(label, value)`, in the format's own order. A field the format has no
    /// name for still appears — labelled positionally rather than dropped.
    ///
    /// Values are **raw**: exactly the bytes the document holds, control
    /// characters and all. Painting them is what makes them safe
    /// ([`crate::render::visible`]), so that copying one yields the real value
    /// rather than the dotted display form.
    pub fields: Vec<(String, String)>,
}
