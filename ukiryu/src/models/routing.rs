//! Source: lib/ukiryu/models/routing.rb (`Ukiryu::Models::Routing`).
//!
//! A routing table maps command names (Symbols in Ruby) to targets, with an
//! optional parent table consulted on misses. Handles share state like the
//! Ruby objects: `child(name)` returns the same cached child each time, and
//! a child sees later changes to its parent. Children are owned by their
//! parent; a child keeps a weak link back to it.
use super::value::Value;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

#[derive(Debug, Default)]
struct Node {
    table: Vec<(String, Value)>,
    parent: Weak<RefCell<Node>>,
    children: Vec<(String, Routing)>,
}

/// `Ukiryu::Models::Routing`.
#[derive(Clone, Debug, Default)]
pub struct Routing(Rc<RefCell<Node>>);

impl PartialEq for Routing {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) || self.0.borrow().table == other.0.borrow().table
    }
}

fn symbolize(table: &Value) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    if let Value::Hash(pairs) = table {
        for (k, v) in pairs {
            let key = k.to_ruby_s();
            if let Some(slot) = out.iter_mut().find(|(existing, _)| *existing == key) {
                slot.1 = v.clone();
            } else {
                out.push((key, v.clone()));
            }
        }
    }
    out
}

impl Routing {
    /// `Routing.new(table)` (keys become Symbols).
    pub fn new(table: &Value) -> Routing {
        Routing(Rc::new(RefCell::new(Node {
            table: symbolize(table),
            ..Node::default()
        })))
    }

    /// `Routing.new(table, parent: parent)`.
    pub fn with_parent(table: &Value, parent: &Routing) -> Routing {
        let r = Routing::new(table);
        r.0.borrow_mut().parent = Rc::downgrade(&parent.0);
        r
    }

    /// `#parent`
    pub fn parent(&self) -> Option<Routing> {
        self.0.borrow().parent.upgrade().map(Routing)
    }

    /// `#table` (Symbol keys, as names).
    pub fn table(&self) -> Vec<(String, Value)> {
        self.0.borrow().table.clone()
    }

    /// `#resolve(command_name)`: own table, then the parent chain.
    pub fn resolve(&self, command_name: &str) -> Option<Value> {
        let own = self
            .0
            .borrow()
            .table
            .iter()
            .find(|(k, _)| k == command_name)
            .map(|(_, v)| v.clone())
            .filter(Value::truthy);
        own.or_else(|| self.parent().and_then(|p| p.resolve(command_name)))
    }

    /// `#resolve_path(path)`
    pub fn resolve_path(&self, path: &[&str]) -> Vec<Value> {
        let Some((first, rest)) = path.split_first() else {
            return Vec::new();
        };
        let Some(target) = self.resolve(first) else {
            return Vec::new();
        };
        let mut out = vec![target];
        if !rest.is_empty() {
            out.extend(self.child(first).resolve_path(rest));
        }
        out
    }

    /// `#include?(command_name)`
    pub fn includes(&self, command_name: &str) -> bool {
        self.0.borrow().table.iter().any(|(k, _)| k == command_name)
            || self.parent().is_some_and(|p| p.includes(command_name))
    }

    /// `#child(command_name)`: the cached child routing for a subcommand level.
    pub fn child(&self, command_name: &str) -> Routing {
        if let Some((_, c)) = self
            .0
            .borrow()
            .children
            .iter()
            .find(|(k, _)| k == command_name)
        {
            return c.clone();
        }
        let child = Routing::with_parent(&Value::Hash(Vec::new()), self);
        self.0
            .borrow_mut()
            .children
            .push((command_name.to_owned(), child.clone()));
        child
    }

    /// `#merge!(other)`
    pub fn merge(&self, other: &Value) -> &Self {
        let incoming = symbolize(other);
        let mut node = self.0.borrow_mut();
        for (k, v) in incoming {
            if let Some(slot) = node.table.iter_mut().find(|(existing, _)| *existing == k) {
                slot.1 = v;
            } else {
                node.table.push((k, v));
            }
        }
        self
    }

    /// `#keys` (sorted Strings).
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .0
            .borrow()
            .table
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        keys
    }

    /// `#empty?`
    pub fn is_empty(&self) -> bool {
        self.0.borrow().table.is_empty()
    }

    /// `#size`
    pub fn size(&self) -> usize {
        self.0.borrow().table.len()
    }

    /// `#to_h` (String keys).
    pub fn to_h(&self) -> Value {
        Value::Hash(
            self.0
                .borrow()
                .table
                .iter()
                .map(|(k, v)| (Value::String(k.clone()), v.clone()))
                .collect(),
        )
    }

    /// `#circular?`: whether the parent chain loops back.
    pub fn is_circular(&self) -> bool {
        let mut seen = vec![Rc::as_ptr(&self.0)];
        let mut current = self.parent();
        while let Some(r) = current {
            let ptr = Rc::as_ptr(&r.0);
            if seen.contains(&ptr) {
                return true;
            }
            seen.push(ptr);
            current = r.parent();
        }
        false
    }

    /// `#to_s`
    pub fn to_ruby_s(&self) -> String {
        let node = self.0.borrow();
        if node.table.is_empty() {
            return "(empty)".into();
        }
        node.table
            .iter()
            .map(|(k, v)| format!("{k} => {}", v.to_ruby_s()))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Whether two handles are the same Ruby object (`equal?`).
    pub fn same_object(&self, other: &Routing) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
