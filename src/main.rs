use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, Message, Notification, Response};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    Position, PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, Url,
};

const DOT_COMMANDS: &[&str] = &[
    ".tran", ".ac", ".dc", ".op", ".noise", ".tf", ".disto", ".four", ".include", ".inc", ".lib",
    ".endl", ".param", ".global", ".options", ".option", ".temp", ".step", ".nodeset", ".ic",
    ".func", ".meas", ".measure", ".save", ".probe", ".print", ".plot", ".connect", ".if",
    ".elseif", ".else", ".endif", ".data", ".enddata", ".end", ".alter", ".subckt", ".ends",
    ".model", ".statistics", ".process", ".section", ".endsection",
];

// Block directives that must be balanced: (opener, closer).
const BLOCK_PAIRS: &[(&str, &str)] = &[(".subckt", ".ends"), (".lib", ".endl"), (".if", ".endif")];

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (conn, io) = Connection::stdio();

    let caps = serde_json::to_value(ServerCapabilities {
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".into(), " ".into()]),
            ..Default::default()
        }),
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        ..Default::default()
    })?;

    conn.initialize(caps)?;

    let mut docs: HashMap<String, String> = HashMap::new();

    for msg in &conn.receiver {
        match msg {
            Message::Request(req) => {
                if conn.handle_shutdown(&req)? {
                    break;
                }
                if req.method == "textDocument/completion" {
                    let params: CompletionParams = serde_json::from_value(req.params.clone())?;
                    let uri = params.text_document_position.text_document.uri.to_string();
                    let items = docs.get(&uri).map(|t| completions(t)).unwrap_or_default();
                    let result = serde_json::to_value(CompletionResponse::Array(items))?;
                    conn.sender
                        .send(Message::Response(Response::new_ok(req.id, result)))?;
                }
            }
            Message::Notification(notif) => match notif.method.as_str() {
                "textDocument/didOpen" => {
                    let p: DidOpenTextDocumentParams = serde_json::from_value(notif.params)?;
                    let uri = p.text_document.uri;
                    docs.insert(uri.to_string(), p.text_document.text.clone());
                    publish_diagnostics(&conn, uri, &p.text_document.text)?;
                }
                "textDocument/didChange" => {
                    let p: DidChangeTextDocumentParams = serde_json::from_value(notif.params)?;
                    let uri = p.text_document.uri;
                    if let Some(change) = p.content_changes.into_iter().last() {
                        docs.insert(uri.to_string(), change.text.clone());
                        publish_diagnostics(&conn, uri, &change.text)?;
                    }
                }
                _ => {}
            },
            Message::Response(_) => {}
        }
    }

    io.join()?;
    Ok(())
}

fn publish_diagnostics(
    conn: &Connection,
    uri: Url,
    text: &str,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let params = PublishDiagnosticsParams {
        uri,
        diagnostics: diagnostics(text),
        version: None,
    };
    conn.sender.send(Message::Notification(Notification::new(
        "textDocument/publishDiagnostics".to_string(),
        params,
    )))?;
    Ok(())
}

fn closer_for(opener: &str) -> Option<&'static str> {
    BLOCK_PAIRS.iter().find(|(o, _)| *o == opener).map(|(_, c)| *c)
}

fn opener_for(closer: &str) -> Option<&'static str> {
    BLOCK_PAIRS.iter().find(|(_, c)| *c == closer).map(|(o, _)| *o)
}

fn line_diagnostic(line_no: usize, line: &str, message: String) -> Diagnostic {
    Diagnostic {
        range: Range {
            start: Position::new(line_no as u32, 0),
            end: Position::new(line_no as u32, line.len() as u32),
        },
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("spice-lsp".to_string()),
        message,
        ..Default::default()
    }
}

// Single-pass scan tracking a stack of open blocks (.subckt/.ends,
// .lib/.endl, .if.../.endif). Bare `.lib` (no argument) opens a block;
// `.lib <path>` is treated as a library reference, not a block opener,
// since that's by far the more common real-world usage.
fn diagnostics(text: &str) -> Vec<Diagnostic> {
    let mut stack: Vec<(&'static str, usize, usize)> = Vec::new();
    let mut diags = Vec::new();

    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('*') {
            continue;
        }
        let mut words = trimmed.split_whitespace();
        let Some(first) = words.next() else { continue };
        let directive = first.to_ascii_lowercase();

        if directive == ".subckt" || directive == ".if" {
            let opener = BLOCK_PAIRS
                .iter()
                .find(|(o, _)| *o == directive)
                .map(|(o, _)| *o)
                .unwrap();
            stack.push((opener, line_no, line.len()));
        } else if directive == ".lib" && words.next().is_none() {
            stack.push((".lib", line_no, line.len()));
        } else if let Some(expected_opener) = opener_for(&directive) {
            match stack.last() {
                Some((top, ..)) if *top == expected_opener => {
                    stack.pop();
                }
                _ => diags.push(line_diagnostic(
                    line_no,
                    line,
                    format!("'{}' has no matching '{}'", first, expected_opener),
                )),
            }
        }
    }

    for (opener, line_no, line_len) in stack {
        let closer = closer_for(opener).unwrap_or("");
        diags.push(Diagnostic {
            range: Range {
                start: Position::new(line_no as u32, 0),
                end: Position::new(line_no as u32, line_len as u32),
            },
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("spice-lsp".to_string()),
            message: format!("'{}' is never closed (missing '{}')", opener, closer),
            ..Default::default()
        });
    }

    diags
}

// Dot-command keywords plus, scanning the document, any `.subckt`/`.model`
// names it defines (so referencing them elsewhere in the file autocompletes).
fn completions(text: &str) -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = DOT_COMMANDS
        .iter()
        .map(|kw| CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        })
        .collect();

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('*') {
            continue;
        }
        let mut words = line.split_whitespace();
        let Some(first) = words.next() else { continue };
        match first.to_ascii_lowercase().as_str() {
            ".subckt" => {
                if let Some(name) = words.next() {
                    let params: Vec<&str> = words.collect();
                    items.push(CompletionItem {
                        label: name.to_string(),
                        kind: Some(CompletionItemKind::FUNCTION),
                        detail: Some(format!(".subckt {} {}", name, params.join(" "))),
                        ..Default::default()
                    });
                }
            }
            ".model" => {
                if let (Some(name), Some(model_type)) = (words.next(), words.next()) {
                    items.push(CompletionItem {
                        label: name.to_string(),
                        kind: Some(CompletionItemKind::CLASS),
                        detail: Some(format!(".model {} {}", name, model_type)),
                        ..Default::default()
                    });
                }
            }
            _ => {}
        }
    }

    items
}
