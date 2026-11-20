/// Only anonymous Ruby class inspect addresses vary per process. Do not
/// mask named classes, ordinary hexadecimal values, members or context.
pub fn canonical_addresses(text: &str) -> String {
    const PREFIX: &str = "#<Class:0x";
    let mut result = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(PREFIX) {
        let after = &rest[start + PREFIX.len()..];
        let length = after.bytes().take_while(u8::is_ascii_hexdigit).count();
        result.push_str(&rest[..start + PREFIX.len()]);
        if length > 0 && after[length..].starts_with('>') {
            result.push_str("<address>");
            rest = &after[length..];
        } else {
            rest = after;
        }
    }
    result.push_str(rest);
    result
}
