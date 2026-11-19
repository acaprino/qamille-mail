use super::*;

impl MeetingService {
    /// Keep a memory fallback if the disk cannot accept either the journal or manifest.
    pub(super) fn finish_job(&self, checkpoint: MeetingJobCheckpoint) {
        let Ok(_gate) = self.job_gate.lock() else {
            return;
        };
        let key = (checkpoint.meeting_id, checkpoint.job_id.clone());
        if let Ok(mut pending) = self.pending_results.lock() {
            pending.insert(key, checkpoint.clone());
        }
        let _ = self.repository.save_job_result(&checkpoint);
        if let Ok(detail) = self.commit_job_result(&checkpoint) {
            // The worker still owns the lease: it is released only after this returns.
            let detail = self.settle_after_commit(detail, &checkpoint, true);
            let _ = self.project(&detail);
            self.clear_result(&checkpoint);
        }
    }

    /// After a transcript result is committed, apply the decision on the audio and return
    /// the meeting as it is now. A problem with the audio leaves the audio alone; it never
    /// undoes or fails the commit.
    ///
    /// `commit_job_result` also returns `Ok` for a checkpoint it did not apply: the job was
    /// cancelled meanwhile, or the content changed during the run and the job became
    /// `failed`. No transcript was saved then, and an older complete transcript must not
    /// let the audio go. So the audio is judged only when this very job is `completed` in
    /// the committed detail; a replay after a crash finds it already `completed` and heals.
    fn settle_after_commit(
        &self,
        committed: MeetingDetail,
        checkpoint: &MeetingJobCheckpoint,
        lease_held: bool,
    ) -> MeetingDetail {
        let saved = matches!(checkpoint.output, Some(MeetingJobOutput::Transcript(_)))
            && committed
                .jobs
                .iter()
                .any(|job| job.id == checkpoint.job_id && job.state == "completed");
        if !saved {
            return committed;
        }
        let id = committed.meeting.id;
        let _ = self.settle_audio(id, lease_held);
        self.repository.read(id).ok().flatten().unwrap_or(committed)
    }

    pub(super) fn reconcile_job_results(&self, detail: MeetingDetail) -> MeetingDetail {
        self.reconcile_job_results_with(detail, false)
    }

    /// `lease_held`: the caller owns this meeting's job lease (startup recovery), so settling
    /// the audio must not claim it again.
    pub(super) fn reconcile_job_results_with(
        &self,
        mut detail: MeetingDetail,
        lease_held: bool,
    ) -> MeetingDetail {
        if detail.deleted {
            return detail;
        }
        let Ok(_gate) = self.job_gate.lock() else {
            return detail;
        };
        let id = detail.meeting.id;
        let (checkpoints, mut warnings) = match self.repository.job_results(id) {
            Ok(results) => results,
            Err(_) => (
                vec![],
                vec!["Verifica dei risultati AI non riuscita: i file sono conservati.".into()],
            ),
        };
        let durable_ids: std::collections::HashSet<_> =
            checkpoints.iter().map(|c| c.job_id.clone()).collect();
        let mut pending: HashMap<String, MeetingJobCheckpoint> = checkpoints
            .into_iter()
            .map(|checkpoint| (checkpoint.job_id.clone(), checkpoint))
            .collect();
        if let Ok(memory) = self.pending_results.lock() {
            for ((meeting_id, job_id), checkpoint) in memory.iter() {
                if *meeting_id == id {
                    pending.insert(job_id.clone(), checkpoint.clone());
                }
            }
        }
        let mut pending: Vec<_> = pending.into_values().collect();
        pending.sort_by(|a, b| (a.created_at, &a.job_id).cmp(&(b.created_at, &b.job_id)));
        for checkpoint in pending {
            match self.commit_job_result(&checkpoint) {
                Ok(saved) => {
                    detail = self.settle_after_commit(saved, &checkpoint, lease_held);
                    let _ = self.project(&detail);
                    self.clear_result(&checkpoint);
                }
                Err(_) => {
                    let message = if durable_ids.contains(&checkpoint.job_id) {
                        "Risultato AI in attesa di salvataggio: ripristina l'accesso al disco e riapri il meeting. L'audio è conservato."
                    } else {
                        "Risultato AI disponibile in memoria: il salvataggio su disco non è confermato. Mantieni l'app aperta e ripristina l'accesso al disco. L'audio è conservato."
                    };
                    if let Some(job) = detail.jobs.iter_mut().find(|j| j.id == checkpoint.job_id) {
                        if matches!(job.state.as_str(), "queued" | "running" | "interrupted") {
                            job.state = "interrupted".into();
                            job.error = Some(message.into());
                        }
                    }
                    warnings.push(message.into());
                }
            }
        }
        // A free OS lease proves no runtime owns these nonterminal jobs. Detect
        // crashed MCP workers during ordinary polling too, without restarting UI.
        // Not while startup recovery is running: it has not reached every meeting yet, and
        // it re-queues only the jobs it finds unfinished, so a window asking first must not
        // mark them interrupted for it.
        if !self.recovering.load(Ordering::SeqCst)
            && detail
                .jobs
                .iter()
                .any(|j| matches!(j.state.as_str(), "queued" | "running"))
        {
            if let Ok(_lease) = self.repository.claim_job(id) {
                match self.repository.update(id, &mut |saved| {
                    for job in &mut saved.jobs {
                        if matches!(job.state.as_str(), "queued" | "running") {
                            job.state = "interrupted".into();
                            job.error =
                                Some("Elaborazione interrotta: riprova per continuare.".into());
                        }
                    }
                    Ok(())
                }) {
                    Ok(saved) => {
                        detail = saved;
                        let _ = self.project(&detail);
                    }
                    Err(_) => {
                        warnings.push("Elaborazione interrotta: riprova per continuare.".into())
                    }
                }
            }
        }
        if !warnings.is_empty() {
            let notice = detail.recovery_notice.get_or_insert_with(String::new);
            for warning in warnings {
                if !notice.contains(&warning) {
                    if !notice.is_empty() {
                        notice.push('\n');
                    }
                    notice.push_str(&warning);
                }
            }
        }
        detail
    }

    fn clear_result(&self, checkpoint: &MeetingJobCheckpoint) {
        // A failed cleanup is harmless: replay is idempotent by the job's terminal state.
        let _ = self
            .repository
            .clear_job_result(checkpoint.meeting_id, &checkpoint.job_id);
        if let Ok(mut pending) = self.pending_results.lock() {
            pending.remove(&(checkpoint.meeting_id, checkpoint.job_id.clone()));
        }
    }

    fn commit_job_result(&self, checkpoint: &MeetingJobCheckpoint) -> AppResult<MeetingDetail> {
        self.repository.update(checkpoint.meeting_id, &mut |detail| {
            if detail.deleted { return Ok(()); }
            let job = detail.jobs.iter().find(|job| job.id == checkpoint.job_id)
                .ok_or_else(|| AppError::missing("Lavoro del risultato recuperato"))?;
            if matches!(job.state.as_str(), "completed" | "failed" | "cancelled") {
                return Ok(());
            }
            if job.input_revision != checkpoint.input_revision {
                return Err(AppError::storage("Revisione del risultato AI incoerente"));
            }
            if checkpoint.output.as_ref().is_some_and(|output| !matches!(
                (job.kind.as_str(), output),
                ("import", MeetingJobOutput::Audio(_))
                    | ("transcribe", MeetingJobOutput::Transcript(_))
                    | ("summarize", MeetingJobOutput::Summary(_))
            )) {
                return Err(AppError::storage("Tipo del risultato AI incoerente"));
            }
            let mut state = "completed";
            let mut error = None;
            if checkpoint.cancelled {
                state = "cancelled";
            } else if detail.revision != checkpoint.input_revision {
                state = "failed";
                error = Some("Il contenuto è cambiato durante l'elaborazione. Il risultato precedente è conservato; riprova.".into());
            } else if let Some(message) = &checkpoint.error {
                state = "failed";
                error = Some(message.clone());
            } else {
                match &checkpoint.output {
                    Some(MeetingJobOutput::Audio(chunks)) => {
                        detail.capture.chunks = chunks.clone();
                        detail.capture.elapsed_ms = chunks.iter().map(|c| c.end_ms).max().unwrap_or(0);
                        detail.capture.saved_ms = detail.capture.elapsed_ms;
                        detail.capture.state = CaptureState::Saved;
                        detail.meeting.duration_secs = detail.capture.elapsed_ms / 1000;
                    }
                    Some(MeetingJobOutput::Transcript(transcript)) => {
                        let mut transcript = transcript.clone();
                        transcript.id = checkpoint.job_id.clone();
                        transcript.created_at = checkpoint.created_at;
                        detail.meeting.transcript = transcript.segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n");
                        detail.active_transcript = Some(transcript.id.clone());
                        detail.transcripts.push(transcript);
                    }
                    Some(MeetingJobOutput::Summary(summary)) => {
                        let mut summary = summary.clone();
                        summary.id = checkpoint.job_id.clone();
                        summary.created_at = checkpoint.created_at;
                        summary.input_revision = checkpoint.input_revision;
                        summary.transcript_version = checkpoint.transcript_version.clone();
                        detail.meeting.summary = summary.summary.clone();
                        detail.meeting.key_points = summary.key_points.clone();
                        detail.meeting.action_items = summary.actions.iter().map(|a| a.text.clone()).collect();
                        detail.active_summary = Some(summary.id.clone());
                        detail.summaries.push(summary);
                    }
                    None => return Err(AppError::storage("Risultato AI non disponibile")),
                }
                detail.revision += 1;
            }
            if let Some(job) = detail.jobs.iter_mut().find(|job| job.id == checkpoint.job_id) {
                job.state = state.into();
                job.error = error;
                if state == "completed" { job.progress = 100; }
            }
            Ok(())
        })
    }
}
