use std::collections::HashMap;
use std::error::Error;

use lsp_server::{Connection, Message, Notification, Response};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, Location, MarkupContent, MarkupKind, OneOf, Position,
    PublishDiagnosticsParams, Range, ReferenceParams, ServerCapabilities, SymbolKind,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url,
};

// Dot-command directives: (keyword, syntax summary, one-line description).
// Used to drive both completion and hover documentation, so the two can
// never drift out of sync with each other.
const DOT_COMMAND_INFO: &[(&str, &str, &str)] = &[
    (".tran", ".tran Tstep Tstop [Tstart [Tmax]] [UIC]", "Transient analysis over time."),
    (".ac", ".ac dec|oct|lin Np Fstart Fstop", "Small-signal AC frequency sweep."),
    (".dc", ".dc SrcName Vstart Vstop Vincr [SrcName2 ...]", "DC sweep of one or more sources."),
    (".op", ".op", "Compute the DC operating point."),
    (".noise", ".noise V(out) SrcName dec Np Fstart Fstop", "Noise analysis at an output relative to a source."),
    (".tf", ".tf OutputVar SrcName", "Small-signal DC transfer function, input/output resistance."),
    (".disto", ".disto dec|oct|lin Np Fstart Fstop", "Small-signal distortion analysis."),
    (".four", ".four Freq OutputVar", "Fourier analysis of the last transient result."),
    (".include", ".include \"path\"", "Textually insert another file at this point."),
    (".inc", ".inc \"path\"", "Alias for .include."),
    (".lib", ".lib \"path\" [section]  |  .lib ... .endl", "Reference a library file/section, or open an inline library block (bare .lib, closed with .endl)."),
    (".endl", ".endl [name]", "Close a .lib block opened without a path argument."),
    (".param", ".param name=value [name2=value2 ...]", "Define a parameter usable in expressions elsewhere in the file."),
    (".global", ".global node1 [node2 ...]", "Declare global nodes visible inside subcircuits without passing them as ports."),
    (".options", ".options name=value ...", "Set simulator control options (tolerances, limits, ...)."),
    (".option", ".option name=value ...", "Alias for .options."),
    (".temp", ".temp value", "Set the simulation temperature in degrees C."),
    (".step", ".step param name Vstart Vstop Vincr", "Sweep a parameter across multiple simulation runs."),
    (".nodeset", ".nodeset V(node)=value ...", "Suggest an initial node voltage guess to help DC convergence."),
    (".ic", ".ic V(node)=value ...", "Set an initial condition for transient analysis."),
    (".func", ".func name(args) { expression }", "Define a reusable function for use in expressions."),
    (".meas", ".meas TRAN|AC|DC name FIND ... | TRIG ... TARG ...", "Measure a value from simulation results."),
    (".measure", ".measure ...", "Alias for .meas."),
    (".save", ".save V(node) I(device) ...", "Restrict which signals are saved from the simulation."),
    (".probe", ".probe [V(node) ...]", "Mark signals for the waveform viewer (LTspice/PSpice)."),
    (".print", ".print TRAN|AC|DC V(node) ...", "Print simulation results to a text table."),
    (".plot", ".plot TRAN|AC|DC V(node) ...", "Plot simulation results (terminal-based simulators)."),
    (".connect", ".connect node1 node2", "Force two nodes to be electrically identical."),
    (".if", ".if (condition)", "Begin a conditional block of netlist text; closed with .endif."),
    (".elseif", ".elseif (condition)", "Additional branch of an .if block."),
    (".else", ".else", "Fallback branch of an .if block."),
    (".endif", ".endif", "Close an .if/.elseif/.else block."),
    (".data", ".data name var1 var2 ...", "Begin a table of sweep data; closed with .enddata."),
    (".enddata", ".enddata", "Close a .data block."),
    (".end", ".end", "Marks the end of the netlist."),
    (".alter", ".alter [title]", "Modify the circuit and re-run analyses (ngspice)."),
    (".subckt", ".subckt name node1 [node2 ...]", "Begin a subcircuit definition; closed with .ends."),
    (".ends", ".ends [name]", "Close a .subckt definition."),
    (".model", ".model name type (param=value ...)", "Define a device model (e.g. D, NPN, NMOS)."),
    (".statistics", ".statistics", "Begin a statistics block (HSPICE Monte Carlo)."),
    (".process", ".process", "Begin a process-variation block (HSPICE)."),
    (".section", ".section name", "Begin a named section within a library."),
    (".endsection", ".endsection [name]", "Close a .section block."),
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
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        document_formatting_provider: Some(OneOf::Left(true)),
        ..Default::default()
    })?;

    conn.initialize(caps)?;

    let mut docs: HashMap<String, (Url, String)> = HashMap::new();

    for msg in &conn.receiver {
        match msg {
            Message::Request(req) => {
                if conn.handle_shutdown(&req)? {
                    break;
                }
                match req.method.as_str() {
                    "textDocument/completion" => {
                        let params: CompletionParams = serde_json::from_value(req.params.clone())?;
                        let uri = params.text_document_position.text_document.uri.to_string();
                        let items = docs
                            .get(&uri)
                            .map(|(_, text)| completions(text))
                            .unwrap_or_default();
                        let result = serde_json::to_value(CompletionResponse::Array(items))?;
                        conn.sender
                            .send(Message::Response(Response::new_ok(req.id, result)))?;
                    }
                    "textDocument/documentSymbol" => {
                        let params: DocumentSymbolParams = serde_json::from_value(req.params.clone())?;
                        let uri = params.text_document.uri.to_string();
                        let symbols = docs
                            .get(&uri)
                            .map(|(_, text)| document_symbols(text))
                            .unwrap_or_default();
                        let result =
                            serde_json::to_value(DocumentSymbolResponse::Nested(symbols))?;
                        conn.sender
                            .send(Message::Response(Response::new_ok(req.id, result)))?;
                    }
                    "textDocument/hover" => {
                        let params: HoverParams = serde_json::from_value(req.params.clone())?;
                        let pos_params = params.text_document_position_params;
                        let uri = pos_params.text_document.uri;
                        let result = docs
                            .get(&uri.to_string())
                            .and_then(|(_, text)| hover(&uri, text, pos_params.position))
                            .map(serde_json::to_value)
                            .transpose()?
                            .unwrap_or(serde_json::Value::Null);
                        conn.sender
                            .send(Message::Response(Response::new_ok(req.id, result)))?;
                    }
                    "textDocument/definition" => {
                        let params: GotoDefinitionParams = serde_json::from_value(req.params.clone())?;
                        let pos_params = params.text_document_position_params;
                        let uri = pos_params.text_document.uri;
                        let result = docs
                            .get(&uri.to_string())
                            .and_then(|(_, text)| goto_definition(&uri, text, pos_params.position))
                            .map(GotoDefinitionResponse::Scalar)
                            .map(serde_json::to_value)
                            .transpose()?
                            .unwrap_or(serde_json::Value::Null);
                        conn.sender
                            .send(Message::Response(Response::new_ok(req.id, result)))?;
                    }
                    "textDocument/references" => {
                        let params: ReferenceParams = serde_json::from_value(req.params.clone())?;
                        let pos_params = params.text_document_position;
                        let uri = pos_params.text_document.uri;
                        let include_decl = params.context.include_declaration;
                        let locations = docs
                            .get(&uri.to_string())
                            .map(|(_, text)| {
                                find_references(&uri, text, pos_params.position, include_decl)
                            })
                            .unwrap_or_default();
                        let result = serde_json::to_value(locations)?;
                        conn.sender
                            .send(Message::Response(Response::new_ok(req.id, result)))?;
                    }
                    "textDocument/formatting" => {
                        let params: DocumentFormattingParams = serde_json::from_value(req.params.clone())?;
                        let uri = params.text_document.uri.to_string();
                        let edits: Vec<TextEdit> = docs
                            .get(&uri)
                            .and_then(|(_, text)| {
                                let formatted = format_document(text);
                                if formatted == *text {
                                    None
                                } else {
                                    Some(vec![TextEdit {
                                        range: Range {
                                            start: Position::new(0, 0),
                                            end: Position::new(u32::MAX, u32::MAX),
                                        },
                                        new_text: formatted,
                                    }])
                                }
                            })
                            .unwrap_or_default();
                        let result = serde_json::to_value(edits)?;
                        conn.sender
                            .send(Message::Response(Response::new_ok(req.id, result)))?;
                    }
                    _ => {}
                }
            }
            Message::Notification(notif) => match notif.method.as_str() {
                "textDocument/didOpen" => {
                    let p: DidOpenTextDocumentParams = serde_json::from_value(notif.params)?;
                    let uri = p.text_document.uri;
                    docs.insert(uri.to_string(), (uri.clone(), p.text_document.text.clone()));
                    publish_diagnostics(&conn, uri, &p.text_document.text)?;
                }
                "textDocument/didChange" => {
                    let p: DidChangeTextDocumentParams = serde_json::from_value(notif.params)?;
                    let uri = p.text_document.uri;
                    if let Some(change) = p.content_changes.into_iter().last() {
                        docs.insert(uri.to_string(), (uri.clone(), change.text.clone()));
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
        uri: uri.clone(),
        diagnostics: diagnostics(&uri, text),
        version: None,
    };
    conn.sender.send(Message::Notification(Notification::new(
        "textDocument/publishDiagnostics".to_string(),
        params,
    )))?;
    Ok(())
}

// ---------------------------------------------------------------------
// Symbol table: .subckt / .model definitions in a document, plus (one
// level of) definitions pulled in via .include / .inc / .lib <path>.
// ---------------------------------------------------------------------

#[derive(Clone)]
struct SubcktDef {
    name: String,
    params: Vec<String>,
    uri: Url,
    line: usize,
    line_text: String,
    name_col: (usize, usize),
}

#[derive(Clone)]
struct ModelDef {
    name: String,
    model_type: String,
    uri: Url,
    line: usize,
    line_text: String,
    name_col: (usize, usize),
}

#[derive(Default)]
struct SymbolTable {
    subckts: Vec<SubcktDef>,
    models: Vec<ModelDef>,
}

impl SymbolTable {
    fn find_subckt(&self, name: &str) -> Option<&SubcktDef> {
        self.subckts
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    fn find_model(&self, name: &str) -> Option<&ModelDef> {
        self.models
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
    }
}

fn words_with_spans(line: &str) -> Vec<(String, usize, usize)> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in line.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                spans.push((line[s..i].to_string(), s, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        spans.push((line[s..].to_string(), s, line.len()));
    }
    spans
}

fn collect_symbols(uri: &Url, text: &str, table: &mut SymbolTable) {
    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('*') {
            continue;
        }
        let spans = words_with_spans(line);
        let Some((first, _, first_end)) = spans.first() else { continue };
        match first.to_ascii_lowercase().as_str() {
            ".subckt" => {
                if let Some((name, ns, ne)) = spans.get(1) {
                    let params = spans[2..].iter().map(|(w, ..)| w.clone()).collect();
                    table.subckts.push(SubcktDef {
                        name: name.clone(),
                        params,
                        uri: uri.clone(),
                        line: line_no,
                        line_text: line.to_string(),
                        name_col: (*ns, *ne),
                    });
                }
            }
            ".model" => {
                if let (Some((name, ns, ne)), Some((mtype, ..))) = (spans.get(1), spans.get(2)) {
                    table.models.push(ModelDef {
                        name: name.clone(),
                        model_type: mtype.clone(),
                        uri: uri.clone(),
                        line: line_no,
                        line_text: line.to_string(),
                        name_col: (*ns, *ne),
                    });
                }
            }
            _ => {
                let _ = first_end;
            }
        }
    }
}

// Resolve `.include`/`.inc`/`.lib <path>` lines relative to `uri`'s
// directory and read them from disk. One level deep only (an include's own
// includes aren't followed) — keeps this a "minimal" server rather than a
// full project indexer.
fn resolve_includes(uri: &Url, text: &str) -> Vec<(Url, String)> {
    let mut out = Vec::new();
    let Ok(base_path) = uri.to_file_path() else { return out };
    let Some(base_dir) = base_path.parent() else { return out };

    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('*') {
            continue;
        }
        let mut words = trimmed.split_whitespace();
        let Some(first) = words.next() else { continue };
        let directive = first.to_ascii_lowercase();
        if directive != ".include" && directive != ".inc" && directive != ".lib" {
            continue;
        }
        let Some(raw_path) = words.next() else { continue };
        let path_str = raw_path.trim_matches(|c| c == '"' || c == '\'');
        if path_str.is_empty() {
            continue;
        }
        let candidate = base_dir.join(path_str);
        if let Ok(contents) = std::fs::read_to_string(&candidate) {
            if let Ok(include_uri) = Url::from_file_path(&candidate) {
                out.push((include_uri, contents));
            }
        }
    }
    out
}

fn build_symbol_table(uri: &Url, text: &str) -> SymbolTable {
    let mut table = SymbolTable::default();
    collect_symbols(uri, text, &mut table);
    for (inc_uri, inc_text) in resolve_includes(uri, text) {
        collect_symbols(&inc_uri, &inc_text, &mut table);
    }
    table
}

fn is_numberish(s: &str) -> bool {
    let s = s.trim_start_matches(['+', '-']);
    s.chars()
        .next()
        .map(|c| c.is_ascii_digit() || c == '.')
        .unwrap_or(false)
}

// The last positional (non key=value) token on a component-card line,
// before any `key=value` parameters start. For `X<name> n1 n2 ... subckt`
// this is the subcircuit reference; for `D1/Q1/M1/... n1 n2 model` it's
// the model reference. Numeric-looking tokens (plain values) are excluded.
fn last_positional_ref(line: &str) -> Option<(String, usize, usize)> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('*') || trimmed.starts_with('.') || trimmed.is_empty() {
        return None;
    }
    let spans = words_with_spans(line);
    if spans.len() < 2 {
        return None;
    }
    let mut end_idx = spans.len();
    for (i, (w, ..)) in spans.iter().enumerate().skip(1) {
        if w.contains('=') {
            end_idx = i;
            break;
        }
    }
    if end_idx <= 1 {
        return None;
    }
    let (word, start, end) = &spans[end_idx - 1];
    if is_numberish(word) {
        return None;
    }
    Some((word.clone(), *start, *end))
}

fn identifier_at(line: &str, character: usize) -> Option<(String, usize, usize)> {
    let bytes = line.as_bytes();
    if character > bytes.len() {
        return None;
    }
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'#' | b'\'');
    // Allow the cursor to sit right after the identifier too.
    let probe = if character < bytes.len() && is_ident(bytes[character]) {
        character
    } else if character > 0 && is_ident(bytes[character - 1]) {
        character - 1
    } else {
        return None;
    };
    let mut start = probe;
    while start > 0 && is_ident(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = probe + 1;
    while end < bytes.len() && is_ident(bytes[end]) {
        end += 1;
    }
    Some((line[start..end].to_string(), start, end))
}

fn line_at(text: &str, line_no: u32) -> Option<&str> {
    text.lines().nth(line_no as usize)
}

// LSP positions are UTF-16 code-unit offsets within a line, not byte
// offsets. SPICE netlists are overwhelmingly ASCII (where the two coincide),
// but these conversions keep the server spec-correct for the rare
// non-ASCII comment/label.
fn byte_to_utf16(line: &str, byte_offset: usize) -> u32 {
    let clamped = byte_offset.min(line.len());
    line[..clamped].encode_utf16().count() as u32
}

fn utf16_to_byte(line: &str, utf16_offset: usize) -> usize {
    let mut utf16_count = 0usize;
    for (byte_idx, ch) in line.char_indices() {
        if utf16_count >= utf16_offset {
            return byte_idx;
        }
        utf16_count += ch.len_utf16();
    }
    line.len()
}

fn pos_at(line: &str, line_no: usize, byte_offset: usize) -> Position {
    Position::new(line_no as u32, byte_to_utf16(line, byte_offset))
}

fn range_at(line: &str, line_no: usize, byte_start: usize, byte_end: usize) -> Range {
    Range {
        start: pos_at(line, line_no, byte_start),
        end: pos_at(line, line_no, byte_end),
    }
}

// ---------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------

fn closer_for(opener: &str) -> Option<&'static str> {
    BLOCK_PAIRS.iter().find(|(o, _)| *o == opener).map(|(_, c)| *c)
}

fn opener_for(closer: &str) -> Option<&'static str> {
    BLOCK_PAIRS.iter().find(|(_, c)| *c == closer).map(|(o, _)| *o)
}

fn line_diagnostic(line_no: usize, line: &str, severity: DiagnosticSeverity, message: String) -> Diagnostic {
    Diagnostic {
        range: range_at(line, line_no, 0, line.len()),
        severity: Some(severity),
        source: Some("spice-lsp".to_string()),
        message,
        ..Default::default()
    }
}

// Device-letter prefixes whose last positional argument is conventionally a
// `.model` reference (diode, BJT, JFET, MOSFET, MESFET).
const MODEL_REF_PREFIXES: &[char] = &['d', 'q', 'j', 'm', 'z'];

// Single-pass scan tracking a stack of open blocks (.subckt/.ends,
// .lib/.endl, .if.../.endif) plus the current `.subckt` name (for
// scope-aware duplicate-instance detection), and symbol-table-driven checks
// for undefined subcircuit/model references and port-count mismatches.
// Bare `.lib` (no argument) opens a block; `.lib <path>` is a reference.
fn diagnostics(uri: &Url, text: &str) -> Vec<Diagnostic> {
    let mut stack: Vec<(&'static str, usize, String)> = Vec::new();
    let mut scope_stack: Vec<String> = Vec::new();
    // (scope, lowercased designator) -> [(line_no, line_text, original designator)]
    let mut instances: HashMap<(String, String), Vec<(usize, String, String)>> = HashMap::new();
    let mut diags = Vec::new();
    let table = build_symbol_table(uri, text);

    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('*') {
            continue;
        }
        let mut words = trimmed.split_whitespace();
        let Some(first) = words.next() else { continue };
        let directive = first.to_ascii_lowercase();

        if directive == ".subckt" {
            let name = words.next().unwrap_or("").to_string();
            stack.push((".subckt", line_no, line.to_string()));
            scope_stack.push(name);
        } else if directive == ".if" {
            stack.push((".if", line_no, line.to_string()));
        } else if directive == ".lib" && words.next().is_none() {
            stack.push((".lib", line_no, line.to_string()));
        } else if let Some(expected_opener) = opener_for(&directive) {
            match stack.last() {
                Some((top, ..)) if *top == expected_opener => {
                    stack.pop();
                    if expected_opener == ".subckt" {
                        scope_stack.pop();
                    }
                }
                _ => diags.push(line_diagnostic(
                    line_no,
                    line,
                    DiagnosticSeverity::ERROR,
                    format!("'{}' has no matching '{}'", first, expected_opener),
                )),
            }
        } else if !directive.starts_with('.') {
            // Component/instance card: track it for duplicate-name detection
            // (scoped to the innermost enclosing .subckt, if any), then run
            // prefix-specific reference checks.
            let scope_key = scope_stack.last().cloned().unwrap_or_default();
            instances
                .entry((scope_key, directive.clone()))
                .or_default()
                .push((line_no, line.to_string(), first.to_string()));

            let prefix = directive.chars().next().unwrap_or(' ');
            if prefix == 'x' {
                // Instance card: X<name> n1 n2 ... subckt_ref [params]
                if let Some((subckt_name, ..)) = last_positional_ref(line) {
                    match table.find_subckt(&subckt_name) {
                        None => diags.push(line_diagnostic(
                            line_no,
                            line,
                            DiagnosticSeverity::WARNING,
                            format!(
                                "subcircuit '{}' is not defined in this file or its includes",
                                subckt_name
                            ),
                        )),
                        Some(def) => {
                            let spans = words_with_spans(line);
                            // Positional args between the designator (index 0)
                            // and the subckt reference itself are the instance's
                            // connection nodes.
                            let ref_idx = spans
                                .iter()
                                .position(|(w, ..)| w.eq_ignore_ascii_case(&subckt_name))
                                .unwrap_or(spans.len());
                            let node_count = ref_idx.saturating_sub(1);
                            if node_count != def.params.len() {
                                diags.push(line_diagnostic(
                                    line_no,
                                    line,
                                    DiagnosticSeverity::WARNING,
                                    format!(
                                        "'{}' has {} node(s), but .subckt '{}' defines {} port(s)",
                                        first,
                                        node_count,
                                        def.name,
                                        def.params.len()
                                    ),
                                ));
                            }
                        }
                    }
                }
            } else if MODEL_REF_PREFIXES.contains(&prefix) {
                // Instance card whose last positional arg is a .model reference,
                // e.g. `D1 anode cathode D1N4148` or `M1 d g s b NMOS1 L=... W=...`
                if let Some((model_name, ..)) = last_positional_ref(line) {
                    if table.find_model(&model_name).is_none() {
                        diags.push(line_diagnostic(
                            line_no,
                            line,
                            DiagnosticSeverity::WARNING,
                            format!(
                                "model '{}' is not defined in this file or its includes",
                                model_name
                            ),
                        ));
                    }
                }
            }
        }
    }

    for (opener, line_no, line_text) in stack {
        let closer = closer_for(opener).unwrap_or("");
        diags.push(Diagnostic {
            range: range_at(&line_text, line_no, 0, line_text.len()),
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("spice-lsp".to_string()),
            message: format!("'{}' is never closed (missing '{}')", opener, closer),
            ..Default::default()
        });
    }

    for occurrences in instances.values() {
        if occurrences.len() < 2 {
            continue;
        }
        let (first_line, ..) = &occurrences[0];
        for (line_no, line_text, name) in occurrences.iter().skip(1) {
            diags.push(line_diagnostic(
                *line_no,
                line_text,
                DiagnosticSeverity::WARNING,
                format!(
                    "duplicate instance name '{}' (first defined on line {})",
                    name,
                    first_line + 1
                ),
            ));
        }
    }

    diags
}

// ---------------------------------------------------------------------
// Completion
// ---------------------------------------------------------------------

// Dot-command keywords plus, scanning the document, any `.subckt`/`.model`
// names it defines (so referencing them elsewhere in the file autocompletes).
fn completions(text: &str) -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = DOT_COMMAND_INFO
        .iter()
        .map(|(kw, syntax, doc)| CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(syntax.to_string()),
            documentation: Some(lsp_types::Documentation::String(doc.to_string())),
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

// ---------------------------------------------------------------------
// Document symbols (outline)
// ---------------------------------------------------------------------

#[allow(deprecated)]
fn document_symbols(text: &str) -> Vec<DocumentSymbol> {
    let mut table = SymbolTable::default();
    // Outline is document-local only; no need for a real uri here.
    let dummy = Url::parse("file:///outline").unwrap();
    collect_symbols(&dummy, text, &mut table);

    let mut symbols = Vec::new();
    for s in &table.subckts {
        let range = range_at(&s.line_text, s.line, 0, s.line_text.len());
        let selection_range = range_at(&s.line_text, s.line, s.name_col.0, s.name_col.1);
        symbols.push(DocumentSymbol {
            name: s.name.clone(),
            detail: Some(format!("ports: {}", s.params.join(", "))),
            kind: SymbolKind::FUNCTION,
            tags: None,
            deprecated: None,
            range,
            selection_range,
            children: None,
        });
    }
    for m in &table.models {
        let range = range_at(&m.line_text, m.line, 0, m.line_text.len());
        let selection_range = range_at(&m.line_text, m.line, m.name_col.0, m.name_col.1);
        symbols.push(DocumentSymbol {
            name: m.name.clone(),
            detail: Some(format!("model type: {}", m.model_type)),
            kind: SymbolKind::CLASS,
            tags: None,
            deprecated: None,
            range,
            selection_range,
            children: None,
        });
    }
    symbols
}

// ---------------------------------------------------------------------
// Hover
// ---------------------------------------------------------------------

fn hover(uri: &Url, text: &str, position: Position) -> Option<Hover> {
    let line = line_at(text, position.line)?;
    let byte_char = utf16_to_byte(line, position.character as usize);
    let (word, ..) = identifier_at(line, byte_char)?;

    let value = if let Some((kw, syntax, doc)) = DOT_COMMAND_INFO
        .iter()
        .find(|(kw, ..)| kw.eq_ignore_ascii_case(&word))
    {
        format!("**{}**\n\n`{}`\n\n{}", kw, syntax, doc)
    } else {
        let table = build_symbol_table(uri, text);
        if let Some(def) = table.find_subckt(&word) {
            format!("**{}** (subcircuit)\n\nports: `{}`", def.name, def.params.join(", "))
        } else if let Some(def) = table.find_model(&word) {
            format!("**{}** (model, type `{}`)", def.name, def.model_type)
        } else {
            return None;
        }
    };

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: None,
    })
}

// ---------------------------------------------------------------------
// Go to definition (including jumping into `.include`/`.inc`/`.lib` files)
// ---------------------------------------------------------------------

fn goto_definition(uri: &Url, text: &str, position: Position) -> Option<Location> {
    let line = line_at(text, position.line)?;
    let trimmed = line.trim_start();
    let directive = trimmed.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    let is_include_line =
        directive == ".include" || directive == ".inc" || directive == ".lib";

    let byte_char = utf16_to_byte(line, position.character as usize);

    if is_include_line {
        if let Some((path_word, start, end)) = words_with_spans(line).get(1).cloned() {
            if byte_char >= start && byte_char <= end {
                let path_str = path_word.trim_matches(|c| c == '"' || c == '\'');
                if !path_str.is_empty() {
                    if let Ok(base_path) = uri.to_file_path() {
                        if let Some(base_dir) = base_path.parent() {
                            let candidate = base_dir.join(path_str);
                            if candidate.exists() {
                                if let Ok(target_uri) = Url::from_file_path(&candidate) {
                                    return Some(Location {
                                        uri: target_uri,
                                        range: Range {
                                            start: Position::new(0, 0),
                                            end: Position::new(0, 0),
                                        },
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let (word, ..) = identifier_at(line, byte_char)?;
    let table = build_symbol_table(uri, text);

    if let Some(def) = table.find_subckt(&word) {
        return Some(Location {
            uri: def.uri.clone(),
            range: range_at(&def.line_text, def.line, def.name_col.0, def.name_col.1),
        });
    }
    if let Some(def) = table.find_model(&word) {
        return Some(Location {
            uri: def.uri.clone(),
            range: range_at(&def.line_text, def.line, def.name_col.0, def.name_col.1),
        });
    }
    None
}

// ---------------------------------------------------------------------
// Find references
// ---------------------------------------------------------------------

fn find_references(
    uri: &Url,
    text: &str,
    position: Position,
    include_declaration: bool,
) -> Vec<Location> {
    let Some(line) = line_at(text, position.line) else { return Vec::new() };
    let byte_char = utf16_to_byte(line, position.character as usize);
    let Some((word, ..)) = identifier_at(line, byte_char) else {
        return Vec::new();
    };

    let mut locations = Vec::new();
    let table = build_symbol_table(uri, text);

    if include_declaration {
        if let Some(def) = table.find_subckt(&word) {
            locations.push(Location {
                uri: def.uri.clone(),
                range: range_at(&def.line_text, def.line, def.name_col.0, def.name_col.1),
            });
        }
        if let Some(def) = table.find_model(&word) {
            locations.push(Location {
                uri: def.uri.clone(),
                range: range_at(&def.line_text, def.line, def.name_col.0, def.name_col.1),
            });
        }
    }

    let mut search = vec![(uri.clone(), text.to_string())];
    search.extend(resolve_includes(uri, text));

    for (doc_uri, doc_text) in search {
        for (line_no, line) in doc_text.lines().enumerate() {
            if let Some((name, start, end)) = last_positional_ref(line) {
                if name.eq_ignore_ascii_case(&word) {
                    locations.push(Location {
                        uri: doc_uri.clone(),
                        range: range_at(line, line_no, start, end),
                    });
                }
            }
        }
    }

    locations
}

// ---------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------

// Whole-comment-line prefixes (a line consisting entirely of a comment).
const COMMENT_LINE_PREFIXES: &[char] = &['*', ';', '$'];

// Whitespace normalization: collapses runs of whitespace between tokens to
// a single space and trims trailing whitespace, while leaving quoted
// strings, `{...}` expressions, and comments untouched. Deliberately does
// not attempt full columnar table alignment across cards — normalizing
// spacing safely is worth more than a fancier aligner that risks corrupting
// unusual lines.
fn format_document(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        out.push_str(&format_line(line));
        out.push('\n');
    }
    if !text.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

fn format_line(line: &str) -> String {
    let trimmed = line.trim_end();
    let content = trimmed.trim_start();

    if content.is_empty() {
        return String::new();
    }
    if content.starts_with(COMMENT_LINE_PREFIXES) {
        return content.to_string();
    }
    if let Some(rest) = content.strip_prefix('+') {
        let rest = normalize_spacing(rest.trim_start());
        if rest.is_empty() {
            return "+".to_string();
        }
        return format!("+ {}", rest);
    }
    normalize_spacing(content)
}

fn normalize_spacing(content: &str) -> String {
    let mut out = String::new();
    let mut chars = content.char_indices().peekable();
    let mut pending_space = false;
    let mut brace_depth = 0i32;
    let mut in_quote: Option<char> = None;

    while let Some((i, c)) = chars.next() {
        if let Some(q) = in_quote {
            out.push(c);
            if c == q {
                in_quote = None;
            }
            continue;
        }
        if brace_depth > 0 {
            out.push(c);
            match c {
                '{' => brace_depth += 1,
                '}' => brace_depth -= 1,
                _ => {}
            }
            continue;
        }
        if c == '"' || c == '\'' {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            in_quote = Some(c);
            out.push(c);
            continue;
        }
        if c == '{' {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            brace_depth = 1;
            out.push(c);
            continue;
        }
        // Inline comments run to the end of the line; copy verbatim.
        if c == ';' || c == '$' {
            if pending_space {
                out.push(' ');
            }
            out.push_str(&content[i..]);
            break;
        }
        if c == '/' && chars.peek().map(|(_, c2)| *c2) == Some('/') {
            if pending_space {
                out.push(' ');
            }
            out.push_str(&content[i..]);
            break;
        }
        if c.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(c);
    }
    out
}
