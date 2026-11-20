//! Recover the scanner context lost in Yeptris's generic block-map error.
use yeptris::event::{EventKind, Puller};

pub(super) fn mapping_value(text: &str, message: &str) -> Option<String> {
    let (line, column) = message
        .strip_prefix("unexpected parser state (line ")?
        .strip_suffix(')')?
        .split_once(", column ")?;
    let line: usize = line.parse().ok()?;
    let column: usize = column.parse().ok()?;
    // Use Unicode columns and one line break for CRLF, like the engine marks.
    let normalized = text.replace("\r\n", "\n");
    let mut marked = normalized
        .split('\n')
        .nth(line.checked_sub(1)?)?
        .chars()
        .skip(column.checked_sub(1)?);
    if marked.next()? != ':' || !marked.next().is_none_or(|c| matches!(c, ' ' | '\t' | '\r')) {
        return None;
    }
    // A colon in a flow collection has a different parser context. Only
    // recover the block mapping's value separator; retain every other error.
    let mut frames = Vec::new();
    let mut parser = Puller::new(normalized.as_bytes()).ok()?;
    for event in parser.events() {
        let Ok(event) = event else { break };
        match event.kind() {
            EventKind::MappingStart | EventKind::SequenceStart => {
                frames.push((event.kind(), event.flow()));
            }
            EventKind::MappingEnd | EventKind::SequenceEnd => {
                frames.pop();
            }
            _ => {}
        }
    }
    if frames.last() != Some(&(EventKind::MappingStart, false))
        || frames.iter().any(|(_, flow)| *flow)
    {
        return None;
    }
    Some(format!("(<unknown>): mapping values are not allowed in this context at line {line} column {column}"))
}
