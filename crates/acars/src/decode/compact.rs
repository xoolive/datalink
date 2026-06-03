//! Compact, human-oriented JSON views for decoded datalink messages.
//!
//! The protocol decoders intentionally expose the full nested wire structure.
//! This module adds a flatter summary envelope for JSONL browsing while keeping
//! the full decode available under `raw_decode` when requested.

use serde_json::{json, Map, Value};

/// Build a compact JSON view from a serialized decoder value.
///
/// When `include_raw` is true, the original value is included as `raw_decode`.
pub fn compact_value(raw: Value, include_raw: bool) -> Value {
    if raw.get("X25").is_some() || raw.get("Acars").is_some() || raw.get("Xid").is_some() {
        compact_avlc_value(raw, include_raw)
    } else if raw.get("label").is_some() && raw.get("text").is_some() {
        compact_acars_value(raw, include_raw)
    } else {
        let mut out = json!({
            "message_class": "unknown",
            "summary": "Decoded message",
        });
        copy_common(&raw, out.as_object_mut().unwrap());
        maybe_raw(&mut out, raw, include_raw);
        out
    }
}

/// Build a compact JSON view for a serialized `AvlcFrame` value.
pub fn compact_avlc_value(raw: Value, include_raw: bool) -> Value {
    let mut out = json!({
        "bearer": "vdl2",
        "path": "unknown",
        "protocol_stack": ["vdl2", "avlc"],
        "message_class": "unknown",
        "summary": "VDL2 AVLC frame",
        "app": Value::Null,
    });
    let obj = out.as_object_mut().unwrap();
    copy_common(&raw, obj);
    copy_if_present(&raw, obj, "src");
    copy_if_present(&raw, obj, "dst");
    copy_if_present(&raw, obj, "role");
    copy_if_present(&raw, obj, "ag_status");
    copy_if_present(&raw, obj, "lcf");

    if let Some(acars) = raw.get("Acars") {
        obj.insert("path".into(), "acars".into());
        obj.insert("protocol_stack".into(), json!(["vdl2", "avlc", "acars"]));
        obj.insert("message_class".into(), "app_message".into());
        obj.insert("summary".into(), acars_summary(acars).into());
        obj.insert("app".into(), acars_app(acars));
    } else if let Some(x25) = raw.get("X25") {
        let (class, stack, summary, app, transport) = x25_compact(x25);
        obj.insert("path".into(), "atn".into());
        obj.insert("protocol_stack".into(), stack);
        obj.insert("message_class".into(), class.into());
        obj.insert("summary".into(), summary.into());
        obj.insert("app".into(), app);
        obj.insert("transport".into(), transport);
    } else if raw.get("Xid").is_some() {
        obj.insert("path".into(), "xid".into());
        obj.insert("protocol_stack".into(), json!(["vdl2", "avlc", "xid"]));
        obj.insert("message_class".into(), "link_management".into());
        obj.insert(
            "summary".into(),
            "VDL2 XID / ground-station information".into(),
        );
    } else if let Some(lcf) = raw.get("lcf") {
        obj.insert("path".into(), "avlc_ctrl".into());
        obj.insert("message_class".into(), "link_control".into());
        obj.insert(
            "summary".into(),
            format!("AVLC control frame: {}", short_json(lcf)).into(),
        );
    }

    maybe_raw(&mut out, raw, include_raw);
    out
}

/// Build a compact JSON view for a serialized `AcarsMessage` value.
pub fn compact_acars_value(raw: Value, include_raw: bool) -> Value {
    let mut out = json!({
        "bearer": raw.pointer("/metadata/bearer").cloned().unwrap_or_else(|| json!("acars")),
        "path": "acars",
        "protocol_stack": acars_stack(&raw),
        "message_class": acars_message_class(&raw),
        "summary": acars_summary(&raw),
        "app": acars_app(&raw),
    });
    let obj = out.as_object_mut().unwrap();
    copy_common(&raw, obj);
    for key in [
        "tail",
        "label",
        "direction",
        "src",
        "dst",
        "block_id",
        "msg_nb",
        "flight_id",
        "metadata",
    ] {
        copy_if_present(&raw, obj, key);
    }
    maybe_raw(&mut out, raw, include_raw);
    out
}

fn x25_compact(x25: &Value) -> (&'static str, Value, String, Value, Value) {
    let transport = json!({
        "x25": {
            "packet_type": x25.get("packet_type").cloned().unwrap_or(Value::Null),
            "channel": format!("{}/{}", val_u64(x25, "chan_group").unwrap_or(0), val_u64(x25, "chan_num").unwrap_or(0)),
            "sseq": x25.get("sseq").cloned().unwrap_or(Value::Null),
            "rseq": x25.get("rseq").cloned().unwrap_or(Value::Null),
            "more": x25.get("more").cloned().unwrap_or(Value::Null),
        }
    });

    let Some(clnp) = x25.pointer("/inner/clnp_compressed") else {
        return (
            "network_data",
            json!(["vdl2", "avlc", "x25"]),
            format!(
                "X.25 {}",
                x25.get("packet_type")
                    .and_then(Value::as_str)
                    .unwrap_or("packet")
            ),
            Value::Null,
            transport,
        );
    };

    if let Some(idrp) = clnp.pointer("/inner/idrp") {
        let typ = idrp
            .get("bispdu_type")
            .and_then(Value::as_str)
            .unwrap_or("IDRP");
        return (
            "network_keepalive",
            json!(["vdl2", "avlc", "x25", "clnp", "idrp"]),
            format!(
                "IDRP {typ}: seq {} ack {}",
                display_field(idrp, "seq"),
                display_field(idrp, "ack")
            ),
            Value::Null,
            transport,
        );
    }

    let Some(cotp) = clnp
        .pointer("/inner/cotp")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    else {
        return (
            "network_data",
            json!(["vdl2", "avlc", "x25", "clnp"]),
            "X.25 / compressed CLNP data".into(),
            Value::Null,
            transport,
        );
    };

    let tpdu = cotp
        .get("tpdu_type")
        .and_then(Value::as_str)
        .unwrap_or("COTP");
    let mut transport = transport;
    if let Some(m) = transport.as_object_mut() {
        m.insert("cotp".into(), cotp_transport(cotp));
    }

    if let Some(cpdlc) = cotp.get("atn_cpdlc") {
        return (
            "app_message",
            json!(["vdl2", "avlc", "x25", "clnp", "cotp", "ulcs", "atn_b1", "cpdlc"]),
            atn_cpdlc_summary(cpdlc),
            json!({
                "protocol": "atn_b1_cpdlc",
                "standard": "ATN B1",
                "message_id": cpdlc.pointer("/header/msg_id").cloned().unwrap_or(Value::Null),
                "message_ref": cpdlc.pointer("/header/msg_ref").cloned().unwrap_or(Value::Null),
                "elements": compact_cpdlc_elements(cpdlc),
            }),
            transport,
        );
    }

    let class = match tpdu {
        "Data Ack" => "transport_ack",
        "Connect Request" => "connect_request",
        "Connect Confirm" => "connect_confirm",
        "Disconnect Request" | "Disconnect Confirm" => "disconnect",
        "Data" => "transport_data",
        _ => "transport_control",
    };
    (
        class,
        json!(["vdl2", "avlc", "x25", "clnp", "cotp"]),
        cotp_summary(cotp),
        Value::Null,
        transport,
    )
}

fn cotp_transport(cotp: &Value) -> Value {
    json!({
        "type": cotp.get("tpdu_type").cloned().unwrap_or(Value::Null),
        "dst_ref": cotp.get("dst_ref").cloned().unwrap_or(Value::Null),
        "src_ref": cotp.get("src_ref").cloned().unwrap_or(Value::Null),
        "sseq": cotp.get("sseq").cloned().unwrap_or(Value::Null),
        "rseq": cotp.get("rseq").cloned().unwrap_or(Value::Null),
        "credit": cotp.get("credit").cloned().unwrap_or(Value::Null),
        "eot": cotp.get("eot").cloned().unwrap_or(Value::Null),
    })
}

fn cotp_summary(cotp: &Value) -> String {
    match cotp
        .get("tpdu_type")
        .and_then(Value::as_str)
        .unwrap_or("COTP")
    {
        "Data Ack" => format!(
            "COTP Data Ack: acknowledged sequence {}, credit {}",
            display_field(cotp, "rseq"),
            display_field(cotp, "credit")
        ),
        "Data" => "COTP Data TPDU with no decoded ATN application payload".into(),
        typ => format!("COTP {typ}"),
    }
}

fn acars_stack(raw: &Value) -> Value {
    if raw.get("data").and_then(|d| d.get("Arinc622")).is_some() {
        let imi = raw
            .pointer("/data/Arinc622/imi")
            .and_then(Value::as_str)
            .unwrap_or("");
        match imi {
            "AT1" | "CR1" | "CC1" | "DR1" => json!(["acars", "arinc622", "fans1a_cpdlc"]),
            "ADS" => json!(["acars", "arinc622", "ads_c"]),
            _ => json!(["acars", "arinc622"]),
        }
    } else {
        json!(["acars"])
    }
}

fn acars_message_class(raw: &Value) -> &'static str {
    match raw.get("data") {
        Some(Value::String(s)) if s == "None" => "empty_acars",
        Some(Value::Object(_)) => "app_message",
        _ => "app_message",
    }
}

fn acars_app(raw: &Value) -> Value {
    let Some(data) = raw.get("data") else {
        return Value::Null;
    };
    if data == "None" {
        return Value::Null;
    }
    if let Some(arinc) = data.get("Arinc622") {
        let imi = arinc
            .get("imi")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let protocol = match imi {
            "AT1" | "CR1" | "CC1" | "DR1" => "fans1a_cpdlc",
            "ADS" => "ads_c",
            _ => "arinc622",
        };
        return json!({
            "protocol": protocol,
            "standard": "ARINC 622",
            "imi": imi,
            "payload": arinc.get("payload").cloned().unwrap_or(Value::Null),
        });
    }
    if let Some(obj) = data.as_object() {
        if let Some((variant, value)) = obj.iter().next() {
            return json!({"protocol": variant.to_ascii_lowercase(), "payload": value});
        }
    }
    data.clone()
}

fn acars_summary(raw: &Value) -> String {
    let label = raw.get("label").and_then(Value::as_str).unwrap_or("??");
    let tail = raw
        .get("tail")
        .and_then(Value::as_str)
        .unwrap_or("unknown tail");
    if let Some(arinc) = raw.get("data").and_then(|d| d.get("Arinc622")) {
        let imi = arinc
            .get("imi")
            .and_then(Value::as_str)
            .unwrap_or("ARINC 622");
        return format!("ACARS {label} {imi} from/to {tail}");
    }
    match raw.get("data") {
        Some(Value::String(s)) if s == "None" => {
            format!("Empty ACARS label {label} from/to {tail}")
        }
        Some(Value::Object(obj)) => {
            let variant = obj.keys().next().map(String::as_str).unwrap_or("payload");
            format!("ACARS label {label} {variant} from/to {tail}")
        }
        _ => format!("ACARS label {label} from/to {tail}"),
    }
}

fn atn_cpdlc_summary(cpdlc: &Value) -> String {
    let parts: Vec<String> = cpdlc
        .get("elements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(cpdlc_element_summary)
        .collect();
    if parts.is_empty() {
        "ATN B1 CPDLC message".into()
    } else {
        format!("ATN B1 CPDLC: {}", parts.join("; "))
    }
}

fn compact_cpdlc_elements(cpdlc: &Value) -> Value {
    Value::Array(
        cpdlc
            .get("elements")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|e| {
                json!({
                    "id": e.get("id").cloned().unwrap_or(Value::Null),
                    "name": e.get("name").cloned().unwrap_or(Value::Null),
                    "summary": cpdlc_element_summary(e),
                    "body": e.get("body").cloned().unwrap_or(Value::Null),
                })
            })
            .collect(),
    )
}

fn cpdlc_element_summary(e: &Value) -> String {
    let name = e
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("CPDLC element");
    if let Some(text) = e.pointer("/body/free_text").and_then(Value::as_str) {
        return format!("{name}: {text}");
    }
    if let Some(fac) = e.pointer("/body/facility/ICAO").and_then(Value::as_str) {
        return format!("{name}: {fac}");
    }
    if let Some(unit) = e.pointer("/body/icao_unit") {
        let freq = e
            .pointer("/body/frequency")
            .map(short_json)
            .unwrap_or_default();
        return format!("{name}: {} {freq}", short_json(unit));
    }
    if let Some(t) = e.get("template").and_then(Value::as_str) {
        return format!("{name}: {t}");
    }
    name.to_string()
}

fn copy_common(raw: &Value, out: &mut Map<String, Value>) {
    for key in ["timestamp", "frame", "metadata"] {
        copy_if_present(raw, out, key);
    }
}

fn copy_if_present(raw: &Value, out: &mut Map<String, Value>, key: &str) {
    if let Some(v) = raw.get(key) {
        out.insert(key.to_string(), v.clone());
    }
}

fn maybe_raw(out: &mut Value, raw: Value, include_raw: bool) {
    if include_raw {
        out.as_object_mut()
            .unwrap()
            .insert("raw_decode".into(), raw);
    }
}

fn val_u64(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

fn display_field(v: &Value, key: &str) -> String {
    v.get(key).map(short_json).unwrap_or_else(|| "?".into())
}

fn short_json(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "?".into()),
    }
}
