use std::path::Path;
use tree_sitter::Node;

#[derive(Debug)]
pub struct BooleanFinding {
    pub line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub evidence: crate::ByteRange,
}

pub fn boolean_findings(path: &str, text: &str) -> Result<Vec<BooleanFinding>, String> {
    let extension = Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !matches!(
        extension,
        "rs" | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "py"
            | "go"
            | "java"
            | "cs"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "hpp"
            | "scala"
            | "kt"
            | "kts"
            | "zig"
    ) {
        return Ok(Vec::new());
    }
    let language = comment_check::language(path, text).ok_or("Boolean grammar unavailable")?;
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language)
        .map_err(|_| "Boolean grammar incompatible")?;
    let tree = parser
        .parse(text, None)
        .ok_or("Boolean parser returned no tree")?;
    if tree.root_node().has_error() {
        return Ok(Vec::new());
    }
    let is_builtin_shadowed = has_builtin_binding(tree.root_node(), text);
    let mut findings = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if let Some((name, evidence)) = boolean_name(node, text, extension, is_builtin_shadowed) {
            if !source(name, text).starts_with("is")
                && source(name, text) != "_"
                && !is_resolved_external_method(node, source(name, text), text, extension)
            {
                findings.push(BooleanFinding {
                    line: name.start_position().row + 1,
                    start_byte: name.start_byte(),
                    end_byte: name.end_byte(),
                    evidence: crate::ByteRange {
                        start_byte: evidence.start_byte(),
                        end_byte: evidence.end_byte(),
                    },
                });
            }
        }
        for index in (0..node.named_child_count()).rev() {
            if let Some(child) = node.named_child(index as u32) {
                stack.push(child);
            }
        }
    }
    findings.sort_unstable_by_key(|f| (f.line, f.start_byte, f.end_byte));
    findings.dedup_by_key(|f| (f.line, f.start_byte, f.end_byte));
    Ok(findings)
}

fn source<'a>(node: Node<'_>, text: &'a str) -> &'a str {
    &text[node.byte_range()]
}

fn field<'a>(node: Node<'a>, fields: &[&str]) -> Option<Node<'a>> {
    fields
        .iter()
        .find_map(|name| node.child_by_field_name(name))
}

fn first_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    (0..node.named_child_count())
        .filter_map(|i| node.named_child(i as u32))
        .find(|c| c.kind() == kind)
}

fn is_boolean_type(node: Node<'_>, text: &str, extension: &str, is_builtin_shadowed: bool) -> bool {
    let name = source(node, text).trim().trim_start_matches(':').trim();
    match extension {
        "rs" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "zig" => {
            name == "bool" || (extension == "c" && name == "_Bool")
        }
        "ts" | "tsx" | "java" => name == "boolean",
        "go" | "py" => name == "bool" && !is_builtin_shadowed,
        "kt" | "kts" => name == "kotlin.Boolean" || (name == "Boolean" && !is_builtin_shadowed),
        "scala" => name == "scala.Boolean" || (name == "Boolean" && !is_builtin_shadowed),
        _ => false,
    }
}

fn has_builtin_binding(root: Node<'_>, text: &str) -> bool {
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        if matches!(node.kind(), "identifier" | "type_identifier")
            && matches!(source(node, text), "bool" | "Boolean")
        {
            if let Some(parent) = node.parent() {
                let is_name =
                    field(parent, &["name", "pattern", "left"]).is_some_and(|name| name == node);
                if is_name
                    || !matches!(
                        parent.kind(),
                        "type"
                            | "user_type"
                            | "type_annotation"
                            | "parameter"
                            | "parameter_declaration"
                            | "field_declaration"
                            | "var_spec"
                            | "function_declaration"
                            | "function_definition"
                            | "val_definition"
                    )
                {
                    return true;
                }
            }
        }
        if cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return false;
            }
        }
    }
}

fn boolean_name<'a>(
    node: Node<'a>,
    text: &str,
    extension: &str,
    is_builtin_shadowed: bool,
) -> Option<(Node<'a>, Node<'a>)> {
    let kind = node.kind();
    let is_rust = extension == "rs";
    let is_c_family = matches!(extension, "c" | "h" | "cc" | "cpp" | "hpp");
    let is_declaration = matches!(
        kind,
        "let_declaration"
            | "field_declaration"
            | "parameter"
            | "function_item"
            | "function_signature_item"
            | "variable_declarator"
            | "public_field_definition"
            | "required_parameter"
            | "optional_parameter"
            | "function_declaration"
            | "method_definition"
            | "method_signature"
            | "property_signature"
            | "assignment"
            | "typed_parameter"
            | "typed_default_parameter"
            | "function_definition"
            | "var_spec"
            | "const_spec"
            | "parameter_declaration"
            | "short_var_declaration"
            | "formal_parameter"
            | "method_declaration"
            | "property_declaration"
            | "init_declarator"
            | "variable_declaration"
            | "val_definition"
            | "var_definition"
    );
    if !is_declaration {
        return None;
    }
    if is_c_family && kind == "variable_declarator" {
        return None;
    }
    let name = field(node, &["name", "pattern", "left", "declarator"])
        .or_else(|| first_kind(node, "identifier"))?;
    let name = if matches!(name.kind(), "function_declarator" | "init_declarator") {
        field(name, &["declarator"])?
    } else if name.kind() == "expression_list" && name.named_child_count() == 1 {
        name.named_child(0)?
    } else {
        name
    };
    if !matches!(
        name.kind(),
        "identifier" | "field_identifier" | "property_identifier"
    ) {
        return None;
    }
    let type_node = field(node, &["return_type", "returns", "result", "type"]).or_else(|| {
        if kind == "variable_declarator" || (is_c_family && kind == "init_declarator") {
            node.parent().and_then(|p| p.child_by_field_name("type"))
        } else if matches!(extension, "kt" | "kts") {
            first_kind(node, "user_type")
        } else {
            None
        }
    });
    if let Some(type_node) =
        type_node.filter(|t| is_boolean_type(*t, text, extension, is_builtin_shadowed))
    {
        return Some((name, type_node));
    }
    if is_rust && matches!(kind, "function_item" | "function_signature_item") {
        return None;
    }
    let value = field(node, &["value", "right"]).or_else(|| {
        if extension == "cs" || extension == "zig" {
            node.named_child(node.named_child_count().checked_sub(1)? as u32)
        } else {
            None
        }
    });
    value
        .filter(|v| is_boolean_expression(*v, text, extension))
        .map(|value| (name, value))
}

fn is_boolean_expression(node: Node<'_>, text: &str, extension: &str) -> bool {
    let value = if node.kind() == "expression_list" && node.named_child_count() == 1 {
        match node.named_child(0) {
            Some(n) => n,
            None => return false,
        }
    } else {
        node
    };
    if matches!(
        value.kind(),
        "true" | "false" | "boolean_literal" | "boolean"
    ) {
        return matches!(source(value, text), "true" | "false" | "True" | "False");
    }
    if value.kind() == "parenthesized_expression" {
        return value
            .named_child(0)
            .is_some_and(|n| is_boolean_expression(n, text, extension));
    }
    if extension == "py" {
        return value.kind() == "not_operator";
    }
    if matches!(value.kind(), "binary_expression") {
        let operator = field(value, &["operator"])
            .map(|n| source(n, text))
            .or_else(|| {
                let left = value.child_by_field_name("left")?;
                let right = value.child_by_field_name("right")?;
                Some(text[left.end_byte()..right.start_byte()].trim())
            })
            .unwrap_or("");
        let is_js = matches!(extension, "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs");
        let is_proven_operators = is_js || matches!(extension, "rs" | "go" | "java");
        if is_proven_operators
            && matches!(
                operator,
                "==" | "!=" | "===" | "!==" | "<" | ">" | "<=" | ">="
            )
        {
            return true;
        }
        return !is_js
            && matches!(extension, "rs" | "go" | "java")
            && matches!(operator, "&&" | "||");
    }
    if matches!(value.kind(), "unary_expression" | "prefix_unary_expression") {
        return matches!(
            extension,
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "go" | "java"
        ) && source(value, text).trim_start().starts_with('!');
    }
    false
}

fn is_resolved_external_method(node: Node<'_>, name: &str, text: &str, extension: &str) -> bool {
    if extension != "rs" || node.kind() != "function_item" {
        return false;
    }
    let Some(implementation) = node
        .parent()
        .and_then(|p| p.parent())
        .filter(|p| p.kind() == "impl_item")
    else {
        return false;
    };
    let Some(trait_node) = implementation.child_by_field_name("trait") else {
        return false;
    };
    let expected = match name {
        "eq" | "ne" => "PartialEq",
        "lt" | "le" | "gt" | "ge" => "PartialOrd",
        _ => return false,
    };
    resolve_standard_path(implementation, source(trait_node, text), text, 0).is_some_and(|path| {
        path == format!("std::cmp::{expected}") || path == format!("core::cmp::{expected}")
    })
}

fn resolve_standard_path(node: Node<'_>, path: &str, text: &str, depth: usize) -> Option<String> {
    if depth >= 16 {
        return None;
    }
    let path = path.split_whitespace().collect::<String>();
    let is_core_disabled = has_scope_attribute(node, text, "no_core");
    let is_std_disabled = is_core_disabled || has_scope_attribute(node, text, "no_std");
    if let Some(absolute) = path.strip_prefix("::") {
        return match absolute.split("::").next() {
            Some("std") if !is_std_disabled => Some(absolute.to_owned()),
            Some("core") if !is_core_disabled => Some(absolute.to_owned()),
            _ => None,
        };
    }
    let (head, tail) = path.split_once("::").unwrap_or((&path, ""));
    let mut current = Some(node);
    let is_prelude_disabled =
        is_core_disabled || has_scope_attribute(node, text, "no_implicit_prelude");
    while let Some(scope) = current {
        if let Some(parameters) = scope.child_by_field_name("type_parameters") {
            let mut cursor = parameters.walk();
            if parameters
                .named_children(&mut cursor)
                .any(|p| field(p, &["name"]).is_some_and(|n| source(n, text) == head))
            {
                return None;
            }
        }
        if matches!(scope.kind(), "source_file" | "declaration_list" | "block") {
            let mut bindings = Vec::new();
            let mut cursor = scope.walk();
            for item in scope.named_children(&mut cursor) {
                if item.kind() == "use_declaration" {
                    let start = bindings.len();
                    if let Some(argument) = item.child_by_field_name("argument") {
                        collect_imports(argument, "", text, &mut bindings);
                    }
                    if item
                        .prev_named_sibling()
                        .is_some_and(|p| p.kind() == "attribute_item")
                    {
                        for (_, target) in &mut bindings[start..] {
                            *target = None;
                        }
                    }
                } else if matches!(
                    item.kind(),
                    "trait_item"
                        | "type_item"
                        | "struct_item"
                        | "enum_item"
                        | "mod_item"
                        | "extern_crate_declaration"
                ) {
                    if let Some(name) = field(item, &["alias", "name"]) {
                        bindings.push((source(name, text).to_owned(), None));
                    }
                } else if item.kind() == "macro_invocation" {
                    return None;
                }
            }
            let mut matching = bindings.iter().filter(|(name, _)| name == head);
            if let Some((_, target)) = matching.next() {
                if matching.next().is_some() {
                    return None;
                }
                let target = target.as_ref()?;
                let target = if tail.is_empty() {
                    target.clone()
                } else {
                    format!("{target}::{tail}")
                };
                return resolve_standard_path(scope, &target, text, depth + 1);
            }
            if bindings.iter().any(|(name, _)| name == "*") {
                return None;
            }
            if scope.kind() == "source_file"
                || scope.parent().is_some_and(|p| p.kind() == "mod_item")
            {
                break;
            }
        }
        current = scope.parent();
    }
    if is_prelude_disabled
        || (head == "std" && is_std_disabled)
        || (head == "core" && is_core_disabled)
    {
        return None;
    }
    if matches!(head, "std" | "core") {
        return Some(path);
    }
    (tail.is_empty() && matches!(head, "PartialEq" | "PartialOrd"))
        .then(|| format!("core::cmp::{head}"))
}

fn has_scope_attribute(node: Node<'_>, text: &str, name: &str) -> bool {
    let mut current = Some(node);
    while let Some(scope) = current {
        let mut cursor = scope.walk();
        if scope
            .named_children(&mut cursor)
            .any(|item| item.kind() == "inner_attribute_item" && source(item, text).contains(name))
        {
            return true;
        }
        let mut previous = scope.prev_named_sibling();
        while let Some(attribute) = previous.filter(|p| p.kind() == "attribute_item") {
            if source(attribute, text).contains(name) {
                return true;
            }
            previous = attribute.prev_named_sibling();
        }
        current = scope.parent();
    }
    false
}

fn collect_imports(
    node: Node<'_>,
    prefix: &str,
    text: &str,
    bindings: &mut Vec<(String, Option<String>)>,
) {
    let join = |part: &str| {
        if prefix.is_empty() {
            part.to_owned()
        } else {
            format!("{prefix}::{part}")
        }
    };
    match node.kind() {
        "scoped_use_list" => {
            let prefix = node
                .child_by_field_name("path")
                .map(|p| join(source(p, text)))
                .unwrap_or_else(|| prefix.to_owned());
            if let Some(list) = node.child_by_field_name("list") {
                collect_imports(list, &prefix, text, bindings);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_imports(child, prefix, text, bindings);
            }
        }
        "use_as_clause" => {
            if let (Some(path), Some(alias)) = (
                node.child_by_field_name("path"),
                node.child_by_field_name("alias"),
            ) {
                bindings.push((
                    source(alias, text).to_owned(),
                    Some(if source(path, text) == "self" {
                        prefix.to_owned()
                    } else {
                        join(source(path, text))
                    }),
                ));
            }
        }
        "use_wildcard" => bindings.push(("*".into(), None)),
        _ => {
            let path = if source(node, text) == "self" {
                prefix.to_owned()
            } else {
                join(source(node, text))
            };
            if let Some(name) = path.rsplit("::").next() {
                bindings.push((name.to_owned(), Some(path.clone())));
            }
        }
    }
}
