use crate::{
    application::{AppError, AppResult},
    domain::meeting::{Evidence, MeetingDetail},
};
use std::collections::HashSet;

pub fn export_meeting(detail: &MeetingDetail, format: &str) -> AppResult<String> {
    match format {
        "json" => {
            serde_json::to_string_pretty(detail).map_err(|e| AppError::storage(e.to_string()))
        }
        "md" | "markdown" | "txt" | "text" => {
            Ok(render(detail, format == "md" || format == "markdown"))
        }
        _ => Err(AppError::validation(
            "Formato di esportazione non supportato. Usa Markdown, TXT o JSON.",
        )),
    }
}
fn render(detail: &MeetingDetail, markdown: bool) -> String {
    let heading = |level: usize, text: &str| {
        if markdown {
            format!("{} {}\n\n", "#".repeat(level), text)
        } else {
            format!("{text}\n\n")
        }
    };
    let mut out = heading(1, &detail.meeting.title);
    out.push_str(&format!(
        "Durata: {}\n\n",
        time(detail.meeting.duration_secs * 1000)
    ));
    if detail.capture.incomplete || detail.capture.state == crate::domain::meeting::CaptureState::Interrupted {
        out.push_str("REGISTRAZIONE INCOMPLETA — recuperata dai blocchi disponibili; alcune parti possono mancare.\n\n");
        for chunk in &detail.capture.missing_chunks {
            out.push_str(&format!("- Audio non disponibile: {} [{}–{}]\n", chunk.source, time(chunk.start_ms), time(chunk.end_ms)));
        }
        for gap in &detail.capture.gaps {
            out.push_str(&format!("- Intervallo audio mancante: {} [{}–{}]\n", gap.source, time(gap.start_ms), time(gap.end_ms)));
        }
        if !detail.capture.unverified_chunks.is_empty() {
            out.push_str(&format!("- Integrità non verificata: {} blocchi audio senza ricevuta.\n", detail.capture.unverified_chunks.len()));
        }
        out.push('\n');
    }
    if let Some(summary) = detail.summary() {
        out.push_str(&heading(2, "Sintesi"));
        if !summary.complete {
            out.push_str("RISULTATO INCOMPLETO\n\n");
        }
        out.push_str(&summary.summary);
        out.push_str("\n\n");
        if !summary.key_points.is_empty() {
            out.push_str(&heading(2, "Punti chiave"));
            for point in &summary.key_points {
                out.push_str(&format!("- {point}\n"));
            }
            out.push('\n');
        }
        if !summary.decisions.is_empty() {
            out.push_str(&heading(2, "Decisioni"));
            for decision in &summary.decisions {
                out.push_str(&format!(
                    "- {}{}\n",
                    decision.text,
                    references(detail, &decision.sources)
                ));
            }
            out.push('\n');
        }
        if !summary.actions.is_empty() {
            out.push_str(&heading(2, "Attività"));
            for action in &summary.actions {
                out.push_str(&format!(
                    "- [{}] {}",
                    if action.done { "x" } else { " " },
                    action.text
                ));
                if let Some(owner) = &action.owner {
                    out.push_str(&format!(" · Responsabile: {owner}"));
                }
                if let Some(due) = &action.due {
                    out.push_str(&format!(" · Scadenza: {due}"));
                }
                out.push_str(&references(detail, &action.sources));
                out.push('\n');
            }
            out.push('\n');
        }
    } else if !detail.meeting.summary.trim().is_empty() {
        out.push_str(&heading(2, "Sintesi"));
        out.push_str(&detail.meeting.summary);
        out.push_str("\n\n");
    }
    if !detail.notes.is_empty() || !detail.meeting.notes.is_empty() {
        out.push_str(&heading(2, "Appunti"));
    }
    if detail.notes.is_empty() {
        out.push_str(&detail.meeting.notes);
        out.push_str("\n\n");
    } else {
        for note in &detail.notes {
            if let Some(at) = note.at_ms {
                out.push_str(&format!("[{}] ", time(at)));
            }
            out.push_str(&note.text);
            out.push_str("\n\n");
        }
    }
    if detail.transcript().is_some() || !detail.meeting.transcript.is_empty() {
        out.push_str(&heading(2, "Trascrizione"));
    }
    if let Some(transcript) = detail.transcript() {
        out.push_str(&format!(
            "Versione: {} · Modello: {} · Lingua: {}\n\n",
            transcript.id, transcript.model, transcript.language
        ));
        if !transcript.complete {
            out.push_str("TRASCRIZIONE INCOMPLETA\n\n");
        }
        for segment in &transcript.segments {
            out.push_str(&format!(
                "[{}–{}] {}\n",
                time(segment.start_ms),
                time(segment.end_ms),
                segment.text
            ));
        }
    } else {
        out.push_str(&detail.meeting.transcript);
        out.push('\n');
    }
    if let Some(summary) = detail.summary() {
        let mut seen = HashSet::new();
        let cited: Vec<_> = summary
            .decisions
            .iter()
            .flat_map(|value| value.sources.iter())
            .chain(
                summary
                    .actions
                    .iter()
                    .flat_map(|value| value.sources.iter()),
            )
            .filter(|reference| {
                seen.insert((
                    reference.kind.clone(),
                    reference.version.clone(),
                    reference.id.clone(),
                ))
            })
            .collect();
        if !cited.is_empty() {
            out.push('\n');
            out.push_str(&heading(2, "Fonti citate"));
            for reference in cited {
                out.push_str(&heading(
                    3,
                    &format!("{} {}:{}", reference.kind, reference.version, reference.id),
                ));
                match source_text(detail, reference) {
                    Some((text, at)) => {
                        if let Some(at) = at {
                            out.push_str(&format!("[{}] ", time(at)));
                        }
                        out.push_str(text);
                        out.push_str("\n\n");
                    }
                    None => out.push_str("Fonte non disponibile in questo archivio.\n\n"),
                }
            }
        }
    }
    out
}

fn source_text<'a>(
    detail: &'a MeetingDetail,
    reference: &Evidence,
) -> Option<(&'a str, Option<u64>)> {
    match reference.kind.as_str() {
        "segment" => detail
            .transcripts
            .iter()
            .find(|version| version.id == reference.version)
            .and_then(|version| {
                version
                    .segments
                    .iter()
                    .find(|segment| segment.id == reference.id)
            })
            .map(|segment| (segment.text.as_str(), Some(segment.start_ms))),
        "note" => detail
            .notes
            .iter()
            .chain(detail.note_history.iter())
            .find(|note| note.id == reference.id && note.revision.to_string() == reference.version)
            .map(|note| (note.text.as_str(), note.at_ms)),
        "legacy_transcript" => detail
            .legacy_transcript
            .as_deref()
            .or_else(|| {
                (!detail.meeting.transcript.is_empty())
                    .then_some(detail.meeting.transcript.as_str())
            })
            .map(|text| (text, None)),
        "legacy_note" => detail
            .legacy_notes
            .as_deref()
            .or_else(|| (!detail.meeting.notes.is_empty()).then_some(detail.meeting.notes.as_str()))
            .map(|text| (text, None)),
        _ => None,
    }
}

fn references(detail: &MeetingDetail, refs: &[crate::domain::meeting::Evidence]) -> String {
    let texts: Vec<String> = refs
        .iter()
        .map(|r| {
            let at = if r.kind == "segment" {
                detail
                    .transcripts
                    .iter()
                    .find(|v| v.id == r.version)
                    .and_then(|v| v.segments.iter().find(|s| s.id == r.id))
                    .map(|s| s.start_ms)
            } else if r.kind == "note" {
                detail
                    .notes
                    .iter()
                    .chain(detail.note_history.iter())
                    .find(|n| n.id == r.id && n.revision.to_string() == r.version)
                    .and_then(|n| n.at_ms)
            } else {
                None
            };
            match at {
                Some(at) => format!("{} [{}:{}]", time(at), r.version, r.id),
                None => format!("{} {}:{}", r.kind, r.version, r.id),
            }
        })
        .collect();
    if texts.is_empty() {
        String::new()
    } else {
        format!(" (Fonti: {})", texts.join(", "))
    }
}
fn time(ms: u64) -> String {
    let seconds = ms / 1000;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{domain::meeting::*, model::Meeting};
    #[test]
    fn recovered_audio_remains_explicitly_incomplete_in_text_exports() {
        let mut detail = MeetingDetail::legacy(Meeting::default());
        detail.capture.state = CaptureState::Saved;
        detail.capture.incomplete = true;
        detail.capture.missing_chunks.push(AudioChunk { source:"microphone".into(), start_ms:1000, end_ms:2000, ..Default::default() });
        for format in ["markdown", "text"] {
            let output = export_meeting(&detail, format).unwrap();
            assert!(output.contains("REGISTRAZIONE INCOMPLETA"));
            assert!(output.contains("00:00:01–00:00:02"));
        }
    }
    #[test]
    fn export_keeps_partial_status_and_original_version() {
        let mut detail = MeetingDetail::legacy(Meeting {
            title: "Prova".into(),
            ..Default::default()
        });
        detail.transcripts.push(TranscriptVersion {
            id: "v1".into(),
            complete: false,
            segments: vec![TranscriptSegment {
                start_ms: 65000,
                end_ms: 68000,
                text: "Controllare i dati".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        detail.active_transcript = Some("v1".into());
        let text = export_meeting(&detail, "md").unwrap();
        assert!(text.contains("TRASCRIZIONE INCOMPLETA"));
        assert!(text.contains("00:01:05"));
        assert!(text.contains("Versione: v1"));
        assert!(export_meeting(&detail, "pdf").is_err());
    }

    #[test]
    fn exported_sources_include_cited_historical_text_and_note_revision() {
        let mut detail = MeetingDetail::legacy(Meeting::default());
        detail.transcripts.push(TranscriptVersion {
            id: "old-transcript".into(),
            segments: vec![TranscriptSegment {
                id: "s1".into(),
                text: "Testo della versione citata.".into(),
                start_ms: 30_000,
                ..Default::default()
            }],
            ..Default::default()
        });
        detail.transcripts.push(TranscriptVersion {
            id: "current".into(),
            segments: vec![],
            ..Default::default()
        });
        detail.active_transcript = Some("current".into());
        detail.note_history.push(MeetingNote {
            id: "n1".into(),
            revision: 1,
            text: "Nota originale approvata.".into(),
            at_ms: Some(60_000),
        });
        detail.notes.push(MeetingNote {
            id: "n1".into(),
            revision: 2,
            text: "Nota successiva.".into(),
            at_ms: None,
        });
        detail.summaries.push(SummaryVersion {
            id: "summary".into(),
            decisions: vec![Decision {
                text: "Approvato".into(),
                sources: vec![
                    Evidence {
                        kind: "segment".into(),
                        version: "old-transcript".into(),
                        id: "s1".into(),
                    },
                    Evidence {
                        kind: "note".into(),
                        version: "1".into(),
                        id: "n1".into(),
                    },
                ],
            }],
            ..Default::default()
        });
        detail.active_summary = Some("summary".into());
        let text = export_meeting(&detail, "txt").unwrap();
        assert!(text.contains("Fonti citate"));
        assert!(text.contains("Testo della versione citata."));
        assert!(text.contains("[00:01:00] Nota originale approvata."));
        assert!(text.contains("old-transcript:s1"));
    }
}
