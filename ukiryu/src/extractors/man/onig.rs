//! A small backtracking matcher for the fixed Onigmo patterns this module
//! ports verbatim from Ruby's `date` extension and from
//! `Ukiryu::ManPageParser`.
//!
//! It is not a general regex engine. It supports exactly the syntax those
//! patterns use: literals, escapes (`\d \D \s \S \w \b \A \z` and escaped
//! punctuation), bracket classes with POSIX `[:alpha:]`/`[:alnum:]`, capturing
//! and non-capturing groups, alternation, greedy and possessive quantifiers
//! (`* + ? {n} {n,m}`), single-character negative look-behind, negative
//! look-ahead and the `(?-i:...)` scoped flag. Matching follows Onigmo's
//! leftmost, first-alternative-wins backtracking order and works on `char`
//! positions, as Ruby's `MatchData#begin`/`#end` do for UTF-8 strings.
//!
//! Character semantics match Ruby on UTF-8 strings: `\d`, `\s` and `\w` are
//! ASCII-only; `[[:alpha:]]`/`[[:alnum:]]` and the `\b` word test are
//! Unicode-aware. Case-insensitive matching folds ASCII letters plus the two
//! non-ASCII single-character folds that reach ASCII (`U+017F` LATIN SMALL
//! LETTER LONG S to `s`, `U+212A` KELVIN SIGN to `k`). Multi-character case
//! folds (ligatures such as `U+FB00`) are not modelled.

#[derive(Debug)]
enum Node {
    Lit(char, bool),
    Class(Class, bool),
    StartText,
    EndText,
    WordBoundary,
    Group(Option<usize>, Box<Node>),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat {
        node: Box<Node>,
        min: usize,
        max: Option<usize>,
        possessive: bool,
    },
    NegLookBehind(Box<Node>),
    NegLookAhead(Box<Node>),
}

#[derive(Debug)]
struct Class {
    negated: bool,
    items: Vec<Item>,
}

#[derive(Debug)]
enum Item {
    Ch(char),
    Range(char, char),
    Digit,
    NotDigit,
    Space,
    NotSpace,
    Word,
    Alpha,
    Alnum,
}

pub(crate) struct Regex {
    root: Node,
    groups: usize,
}

pub(crate) type Caps = Vec<Option<(usize, usize)>>;

/// A successful match: group 0 is the whole match, positions are `char`
/// indices into the searched text.
pub(crate) struct Match {
    caps: Caps,
}

impl Match {
    pub(crate) fn range(&self, group: usize) -> Option<(usize, usize)> {
        self.caps.get(group).copied().flatten()
    }

    pub(crate) fn text(&self, chars: &[char], group: usize) -> Option<String> {
        self.range(group)
            .map(|(b, e)| chars[b..e].iter().collect::<String>())
    }
}

fn is_ruby_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

fn is_ascii_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Onigmo's Unicode word test used by `\b`.
fn is_unicode_word(c: char) -> bool {
    if c.is_ascii() {
        is_ascii_word(c)
    } else {
        c.is_alphanumeric()
    }
}

fn fold(c: char) -> char {
    match c {
        '\u{017F}' => 's',
        '\u{212A}' => 'k',
        _ => c.to_ascii_lowercase(),
    }
}

impl Item {
    fn matches(&self, c: char) -> bool {
        match self {
            Item::Ch(x) => *x == c,
            Item::Range(a, b) => *a <= c && c <= *b,
            Item::Digit => c.is_ascii_digit(),
            Item::NotDigit => !c.is_ascii_digit(),
            Item::Space => is_ruby_space(c),
            Item::NotSpace => !is_ruby_space(c),
            Item::Word => is_ascii_word(c),
            Item::Alpha => c.is_alphabetic(),
            Item::Alnum => c.is_alphanumeric(),
        }
    }
}

impl Class {
    fn matches(&self, c: char, ci: bool) -> bool {
        let hit = |x: char| self.items.iter().any(|i| i.matches(x));
        let mut found = hit(c);
        if !found && ci {
            let f = fold(c);
            found = hit(f) || hit(f.to_ascii_uppercase());
        }
        found != self.negated
    }
}

struct Parser<'a> {
    src: &'a [char],
    pos: usize,
    groups: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }

    fn eat(&mut self, s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        if self.src[self.pos..].starts_with(&chars) {
            self.pos += chars.len();
            true
        } else {
            false
        }
    }

    fn alt(&mut self, ci: bool) -> Node {
        let mut alts = vec![self.concat(ci)];
        while self.peek() == Some('|') {
            self.pos += 1;
            alts.push(self.concat(ci));
        }
        if alts.len() == 1 {
            alts.pop().expect("one alternative")
        } else {
            Node::Alt(alts)
        }
    }

    fn concat(&mut self, ci: bool) -> Node {
        let mut nodes = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom(ci);
            nodes.push(self.quantified(atom));
        }
        Node::Concat(nodes)
    }

    fn quantified(&mut self, atom: Node) -> Node {
        let (min, max) = match self.peek() {
            Some('*') => {
                self.pos += 1;
                (0, None)
            }
            Some('+') => {
                self.pos += 1;
                (1, None)
            }
            Some('?') => {
                self.pos += 1;
                (0, Some(1))
            }
            Some('{') => {
                self.pos += 1;
                let min = self.number();
                let max = if self.eat(",") {
                    Some(self.number())
                } else {
                    Some(min)
                };
                assert!(self.eat("}"), "unterminated interval");
                (min, max)
            }
            _ => return atom,
        };
        let possessive = self.peek() == Some('+');
        if possessive {
            self.pos += 1;
        }
        Node::Repeat {
            node: Box::new(atom),
            min,
            max,
            possessive,
        }
    }

    fn number(&mut self) -> usize {
        let mut n = 0;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            n = n * 10 + d as usize;
            self.pos += 1;
        }
        n
    }

    fn atom(&mut self, ci: bool) -> Node {
        let c = self.peek().expect("atom");
        self.pos += 1;
        match c {
            '(' => {
                if self.eat("?:") {
                    let inner = self.alt(ci);
                    self.close();
                    Node::Group(None, Box::new(inner))
                } else if self.eat("?<!") {
                    let inner = self.alt(ci);
                    self.close();
                    Node::NegLookBehind(Box::new(inner))
                } else if self.eat("?!") {
                    let inner = self.alt(ci);
                    self.close();
                    Node::NegLookAhead(Box::new(inner))
                } else if self.eat("?-i:") {
                    let inner = self.alt(false);
                    self.close();
                    Node::Group(None, Box::new(inner))
                } else {
                    self.groups += 1;
                    let index = self.groups;
                    let inner = self.alt(ci);
                    self.close();
                    Node::Group(Some(index), Box::new(inner))
                }
            }
            '[' => Node::Class(self.class(), ci),
            '\\' => self.escape(ci),
            '.' => panic!("unescaped '.' is not used by the ported patterns"),
            _ => Node::Lit(c, ci),
        }
    }

    fn close(&mut self) {
        assert!(self.eat(")"), "unterminated group");
    }

    fn escape(&mut self, ci: bool) -> Node {
        let c = self.peek().expect("escape");
        self.pos += 1;
        let class = |item| {
            Node::Class(
                Class {
                    negated: false,
                    items: vec![item],
                },
                false,
            )
        };
        match c {
            'd' => class(Item::Digit),
            'D' => class(Item::NotDigit),
            's' => class(Item::Space),
            'S' => class(Item::NotSpace),
            'w' => class(Item::Word),
            'b' => Node::WordBoundary,
            'A' => Node::StartText,
            'z' => Node::EndText,
            _ => Node::Lit(c, ci),
        }
    }

    fn class(&mut self) -> Class {
        let negated = self.eat("^");
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.peek().expect("unterminated class");
            if c == ']' && !first {
                self.pos += 1;
                break;
            }
            first = false;
            if self.eat("[:alpha:]") {
                items.push(Item::Alpha);
                continue;
            }
            if self.eat("[:alnum:]") {
                items.push(Item::Alnum);
                continue;
            }
            self.pos += 1;
            let lit = if c == '\\' {
                let e = self.peek().expect("class escape");
                self.pos += 1;
                match e {
                    'd' => {
                        items.push(Item::Digit);
                        continue;
                    }
                    's' => {
                        items.push(Item::Space);
                        continue;
                    }
                    _ => e,
                }
            } else {
                c
            };
            if self.peek() == Some('-') && self.src.get(self.pos + 1).is_some_and(|n| *n != ']') {
                self.pos += 1;
                let hi = self.peek().expect("range end");
                self.pos += 1;
                items.push(Item::Range(lit, hi));
            } else {
                items.push(Item::Ch(lit));
            }
        }
        Class { negated, items }
    }
}

impl Regex {
    /// Compiles one of the fixed source patterns. `ci` mirrors the Onigmo
    /// `IGNORECASE` option the source passes for that pattern.
    pub(crate) fn new(pattern: &str, ci: bool) -> Regex {
        let src: Vec<char> = pattern.chars().collect();
        let mut parser = Parser {
            src: &src,
            pos: 0,
            groups: 0,
        };
        let root = parser.alt(ci);
        assert!(parser.pos == src.len(), "trailing pattern text: {pattern}");
        Regex {
            root,
            groups: parser.groups,
        }
    }

    /// `Regexp#match`: leftmost match searching from every start position.
    pub(crate) fn find(&self, text: &[char]) -> Option<Match> {
        for start in 0..=text.len() {
            let mut caps: Caps = vec![None; self.groups + 1];
            let mut end = None;
            if m(&self.root, text, start, &mut caps, &mut |j, _| {
                end = Some(j);
                true
            }) {
                caps[0] = Some((start, end.expect("end")));
                return Some(Match { caps });
            }
        }
        None
    }
}

type Cont<'k> = dyn FnMut(usize, &mut Caps) -> bool + 'k;

fn single(node: &Node, text: &[char], i: usize) -> Option<bool> {
    match node {
        Node::Lit(c, ci) => {
            Some(
                text.get(i)
                    .is_some_and(|t| if *ci { fold(*t) == fold(*c) } else { t == c }),
            )
        }
        Node::Class(class, ci) => Some(text.get(i).is_some_and(|t| class.matches(*t, *ci))),
        _ => None,
    }
}

fn m(node: &Node, text: &[char], i: usize, caps: &mut Caps, k: &mut Cont<'_>) -> bool {
    if let Some(ok) = single(node, text, i) {
        return ok && k(i + 1, caps);
    }
    match node {
        Node::Lit(..) | Node::Class(..) => unreachable!("handled by single"),
        Node::StartText => i == 0 && k(i, caps),
        Node::EndText => i == text.len() && k(i, caps),
        Node::WordBoundary => {
            let before = i > 0 && is_unicode_word(text[i - 1]);
            let after = i < text.len() && is_unicode_word(text[i]);
            before != after && k(i, caps)
        }
        Node::Group(index, inner) => match index {
            None => m(inner, text, i, caps, k),
            Some(g) => {
                let g = *g;
                m(inner, text, i, caps, &mut |j, caps: &mut Caps| {
                    let saved = caps[g];
                    caps[g] = Some((i, j));
                    if k(j, caps) {
                        true
                    } else {
                        caps[g] = saved;
                        false
                    }
                })
            }
        },
        Node::Concat(nodes) => seq(nodes, text, i, caps, k),
        Node::Alt(alts) => {
            for alt in alts {
                let saved = caps.clone();
                if m(alt, text, i, caps, k) {
                    return true;
                }
                *caps = saved;
            }
            false
        }
        Node::Repeat {
            node,
            min,
            max,
            possessive,
        } => {
            if single(node, text, i).is_some() {
                let mut n = 0;
                while max.is_none_or(|x| n < x) && single(node, text, i + n) == Some(true) {
                    n += 1;
                }
                if *possessive {
                    return n >= *min && k(i + n, caps);
                }
                let mut count = n;
                loop {
                    if count < *min {
                        return false;
                    }
                    if k(i + count, caps) {
                        return true;
                    }
                    if count == 0 {
                        return false;
                    }
                    count -= 1;
                }
            }
            assert!(!*possessive, "possessive group repeat is not used");
            rep(node, *min, *max, 0, text, i, caps, k)
        }
        Node::NegLookBehind(inner) => {
            let mut hit = false;
            if i > 0 {
                let mut scratch = caps.clone();
                hit = m(inner, text, i - 1, &mut scratch, &mut |j, _| j == i);
            }
            !hit && k(i, caps)
        }
        Node::NegLookAhead(inner) => {
            let mut scratch = caps.clone();
            let hit = m(inner, text, i, &mut scratch, &mut |_, _| true);
            !hit && k(i, caps)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn rep(
    node: &Node,
    min: usize,
    max: Option<usize>,
    count: usize,
    text: &[char],
    i: usize,
    caps: &mut Caps,
    k: &mut Cont<'_>,
) -> bool {
    if max.is_none_or(|x| count < x) {
        let saved = caps.clone();
        if m(node, text, i, caps, &mut |j, caps: &mut Caps| {
            if j == i && count >= min {
                return false;
            }
            rep(node, min, max, count + 1, text, j, caps, k)
        }) {
            return true;
        }
        *caps = saved;
    }
    count >= min && k(i, caps)
}

fn seq(nodes: &[Node], text: &[char], i: usize, caps: &mut Caps, k: &mut Cont<'_>) -> bool {
    match nodes.split_first() {
        None => k(i, caps),
        Some((first, rest)) => m(first, text, i, caps, &mut |j, caps: &mut Caps| {
            seq(rest, text, j, caps, k)
        }),
    }
}
