//! GFM tables: header row, `:---:` delimiter row, body rows, escaped pipes,
//! pipes inside inline code, ragged rows padded/truncated to header arity.
#![deny(unsafe_code)]

use super::ast::{Align, Block, Inline, LinkRefs};
use super::inline::parse_inlines;
use super::scan::{atx, fence_at, indent_of, is_blank, is_thematic_break, quote_start, Ln};

/// True when `lines[i]` is a header row followed by a matching delimiter row;
/// yields the column count.
pub(crate) fn detect(lines: &[Ln], i: usize) -> Option<usize> {
    let head = &lines[i].text;
    if indent_of(head) >= 4 || !head.contains('|') || i + 1 >= lines.len() {
        return None;
    }
    let cols = split_row(head).len();
    let aligns = delimiter_row(&lines[i + 1].text)?;
    if aligns.len() == cols && cols > 0 {
        Some(cols)
    } else {
        None
    }
}

/// Parse a table at `lines[i]`; returns the index after it, or `None` if this
/// is not a table.
pub(crate) fn parse_table(
    lines: &[Ln],
    i: usize,
    refs: &LinkRefs,
    out: &mut Vec<Block>,
) -> Option<usize> {
    let cols = detect(lines, i)?;
    let align = delimiter_row(&lines[i + 1].text)?;
    let head: Vec<Vec<Inline>> = split_row(&lines[i].text)
        .iter()
        .map(|c| parse_inlines(c, refs))
        .collect();
    let mut rows = Vec::new();
    let mut j = i + 2;
    while j < lines.len() {
        let s = &lines[j].text;
        if !is_row(s) {
            break;
        }
        let mut cells = split_row(s);
        cells.truncate(cols);
        while cells.len() < cols {
            cells.push(String::new());
        }
        rows.push(cells.iter().map(|c| parse_inlines(c, refs)).collect());
        j += 1;
    }
    out.push(Block::Table {
        align,
        head,
        rows,
        source_line: lines[i].num,
    });
    Some(j)
}

fn is_row(s: &str) -> bool {
    s.contains('|')
        && !is_blank(s)
        && indent_of(s) < 4
        && !is_thematic_break(s)
        && !quote_start(s)
        && atx(s).is_none()
        && fence_at(s).is_none()
}

/// `| :--- | :---: | ---: |` -> per-column alignment.
pub(crate) fn delimiter_row(s: &str) -> Option<Vec<Align>> {
    if indent_of(s) >= 4 || !s.contains('|') || !s.contains('-') {
        return None;
    }
    let cells = split_row(s);
    if cells.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(cells.len());
    for c in &cells {
        let c = c.trim();
        let left = c.starts_with(':');
        let right = c.ends_with(':') && c.len() > 1;
        let core = &c[usize::from(left)..c.len() - usize::from(right)];
        if core.is_empty() || !core.chars().all(|x| x == '-') {
            return None;
        }
        out.push(match (left, right) {
            (true, true) => Align::Center,
            (true, false) => Align::Left,
            (false, true) => Align::Right,
            (false, false) => Align::None,
        });
    }
    Some(out)
}

fn escaped(cs: &[char], i: usize) -> bool {
    let mut n = 0;
    let mut k = i;
    while k > 0 && cs[k - 1] == '\\' {
        n += 1;
        k -= 1;
    }
    n % 2 == 1
}

fn run_len(cs: &[char], i: usize) -> usize {
    let mut n = 0;
    while i + n < cs.len() && cs[i + n] == '`' {
        n += 1;
    }
    n
}

/// Mark the byte-positions covered by backtick code spans so pipes inside
/// them are not treated as cell delimiters.
fn code_mask(cs: &[char]) -> Vec<bool> {
    let mut mask = vec![false; cs.len()];
    let mut i = 0;
    while i < cs.len() {
        if cs[i] != '`' || escaped(cs, i) {
            i += 1;
            continue;
        }
        let n = run_len(cs, i);
        let mut j = i + n;
        let mut close = None;
        while j < cs.len() {
            if cs[j] == '`' {
                let m = run_len(cs, j);
                if m == n {
                    close = Some(j);
                    break;
                }
                j += m;
            } else {
                j += 1;
            }
        }
        match close {
            Some(j) => {
                for slot in mask.iter_mut().take(j + n).skip(i) {
                    *slot = true;
                }
                i = j + n;
            }
            None => i += n,
        }
    }
    mask
}

/// Split a table row into trimmed cells, honoring `\|` escapes and code spans
/// and dropping the optional leading/trailing pipes.
pub(crate) fn split_row(s: &str) -> Vec<String> {
    let t = s.trim();
    let cs: Vec<char> = t.chars().collect();
    let mask = code_mask(&cs);
    let mut cells: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (i, &c) in cs.iter().enumerate() {
        if c == '|' && !mask[i] && !escaped(&cs, i) {
            cells.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    cells.push(cur);
    if cells.len() > 1 && t.starts_with('|') && cells[0].trim().is_empty() {
        cells.remove(0);
    }
    if cells.len() > 1 && cells.last().map(|c| c.trim().is_empty()).unwrap_or(false) {
        let end_pipe = !cs.is_empty() && cs[cs.len() - 1] == '|' && !escaped(&cs, cs.len() - 1);
        if end_pipe {
            cells.pop();
        }
    }
    cells
        .iter()
        .map(|c| c.replace("\\|", "|").trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_basic_row() {
        assert_eq!(split_row("| a | b |"), vec!["a", "b"]);
        assert_eq!(split_row("a | b"), vec!["a", "b"]);
        assert_eq!(split_row("|---|---|"), vec!["---", "---"]);
    }

    #[test]
    fn escaped_pipe_becomes_literal() {
        let cells = split_row(r"| `--waf-tier free\|pro` | Create a river. |");
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0], "`--waf-tier free|pro`");
    }

    #[test]
    fn pipe_inside_code_span_does_not_split() {
        let cells = split_row("| `a | b` | c |");
        assert_eq!(cells, vec!["`a | b`", "c"]);
    }

    #[test]
    fn unclosed_backtick_still_splits() {
        assert_eq!(split_row("| a ` b | c |"), vec!["a ` b", "c"]);
    }

    #[test]
    fn delimiter_alignments() {
        assert_eq!(
            delimiter_row("| :--- | :---: | ---: | --- |"),
            Some(vec![Align::Left, Align::Center, Align::Right, Align::None])
        );
        assert_eq!(delimiter_row("|---|---|"), Some(vec![Align::None; 2]));
        assert_eq!(delimiter_row("| a | b |"), None);
        assert_eq!(delimiter_row("not a row"), None);
    }

    #[test]
    fn empty_trailing_cell_is_kept_without_pipe() {
        // "a | b |" ends with a pipe -> two cells; "a | b | " likewise.
        assert_eq!(split_row("a | b |"), vec!["a", "b"]);
        assert_eq!(split_row("| a || c |"), vec!["a", "", "c"]);
    }
}
