use super::{cancelled, processing_error, reserve_disk, unique_id, write_wav};
use crate::{
    application::{AppError, AppResult},
    domain::meeting::AudioChunk,
};
use std::{
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

/// Decode packet-by-packet into five-second mono WAV blocks. The caller owns publication.
pub fn import_audio(
    source: &Path,
    folder: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u32),
) -> AppResult<Vec<AudioChunk>> {
    cancelled(cancel)?;
    fs::create_dir_all(folder)?;
    let original = snapshot_original(source, folder, cancel)?;
    progress(10);
    let result = decode_snapshot(&original, folder, cancel, &|value| {
        progress(10 + value * 90 / 100);
    });
    if result.is_err() {
        let _ = fs::remove_file(original);
    }
    result
}

fn snapshot_original(source: &Path, folder: &Path, cancel: &AtomicBool) -> AppResult<PathBuf> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Hold a read-only share while copying: another process cannot modify the source.
        options.share_mode(1);
    }
    let mut input = options.open(source)?;
    let before = input.metadata()?;
    if before.len() == 0 {
        return Err(AppError::new("unsupported_audio", "Il file audio è vuoto."));
    }
    let _space = reserve_disk(folder, before.len())?;
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|value| matches!(value.as_str(), "wav" | "mp3" | "m4a" | "aac" | "flac"))
        .unwrap_or_else(|| "audio".into());
    let original = folder.join(format!("original-{}.{extension}", unique_id()?));
    let staging = original.with_extension(format!("{extension}.part"));
    let result = (|| {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        let mut buffer = [0_u8; 64 * 1024];
        let mut copied = 0_u64;
        loop {
            cancelled(cancel)?;
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
            copied += count as u64;
        }
        let after = input.metadata()?;
        if copied != before.len()
            || after.len() != before.len()
            || before.modified().ok() != after.modified().ok()
        {
            return Err(processing_error(
                "Il file è cambiato durante la copia. Riprova quando è stato salvato.",
            ));
        }
        output.sync_all()?;
        drop(output);
        cancelled(cancel)?;
        fs::rename(&staging, &original)?;
        Ok(original)
    })();
    if result.is_err() {
        let _ = fs::remove_file(staging);
    }
    result
}

fn decode_snapshot(
    source: &Path,
    folder: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u32),
) -> AppResult<Vec<AudioChunk>> {
    let file = File::open(source)?;
    if file.metadata()?.len() == 0 {
        return Err(AppError::new("unsupported_audio", "Il file audio è vuoto."));
    }
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = source.extension().and_then(|v| v.to_str()) {
        hint.with_extension(extension);
    }
    let probe = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|_| {
            AppError::new(
                "unsupported_audio",
                "Formato audio non supportato o file non valido. Usa WAV, MP3, M4A/AAC o FLAC.",
            )
        })?;
    let mut format = probe.format;
    let track = format
        .default_track()
        .ok_or_else(|| AppError::new("unsupported_audio", "Nessuna traccia audio nel file."))?;
    let track_id = track.id;
    let codec = track.codec_params.clone();
    let expected_frames = codec.n_frames;
    let mut decoder = symphonia::default::get_codecs()
        .make(&codec, &DecoderOptions::default())
        .map_err(|_| AppError::new("unsupported_audio", "Il codec del file non è supportato."))?;
    fs::create_dir_all(folder)?;
    let prefix = format!("import-{}", unique_id()?);
    let mut chunks = Vec::new();
    let mut pending = Vec::new();
    let mut sample_rate = 0;
    let mut total_frames = 0_u64;
    let result = (|| {
        loop {
            cancelled(cancel)?;
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => break,
                Err(_) => {
                    return Err(processing_error(
                        "Lettura del file audio interrotta: nessun risultato è stato pubblicato.",
                    ));
                }
            };
            if packet.track_id() != track_id {
                continue;
            }
            let decoded = decoder
                .decode(&packet)
                .map_err(|_| processing_error("Il file contiene un blocco audio danneggiato."))?;
            let spec = *decoded.spec();
            if spec.rate < 8_000
                || spec.rate > 192_000
                || spec.channels.count() == 0
                || spec.channels.count() > 32
            {
                return Err(AppError::new(
                    "unsupported_audio",
                    "Configurazione audio non supportata.",
                ));
            }
            if sample_rate == 0 {
                sample_rate = spec.rate;
            }
            if sample_rate != spec.rate {
                return Err(AppError::new(
                    "unsupported_audio",
                    "Il file cambia frequenza di campionamento durante la traccia.",
                ));
            }
            if decoded.capacity() > spec.rate as usize * 30 {
                return Err(processing_error(
                    "Blocco audio oltre il limite di memoria previsto.",
                ));
            }
            let mut interleaved = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            interleaved.copy_interleaved_ref(decoded);
            let channels = spec.channels.count();
            for frame in interleaved.samples().chunks_exact(channels) {
                let sample = frame.iter().sum::<f32>() / channels as f32;
                if !sample.is_finite() {
                    return Err(processing_error(
                        "Il file contiene campioni audio non validi.",
                    ));
                }
                pending.push(sample.clamp(-1.0, 1.0));
                if pending.len() == sample_rate as usize * 5 {
                    commit_chunk(
                        folder,
                        &prefix,
                        &mut chunks,
                        &pending,
                        sample_rate,
                        total_frames,
                    )?;
                    total_frames += pending.len() as u64;
                    pending.clear();
                    if let Some(expected) = expected_frames {
                        progress(
                            (total_frames.saturating_mul(100) / expected.max(1)).min(99) as u32
                        );
                    }
                }
            }
        }
        cancelled(cancel)?;
        if !pending.is_empty() {
            commit_chunk(
                folder,
                &prefix,
                &mut chunks,
                &pending,
                sample_rate,
                total_frames,
            )?;
            total_frames += pending.len() as u64;
        }
        if chunks.is_empty() {
            return Err(AppError::new(
                "unsupported_audio",
                "Il file non contiene campioni audio decodificabili.",
            ));
        }
        // Container-reported length is advisory, but a large shortfall indicates truncation.
        if let Some(expected) = expected_frames {
            if total_frames + (sample_rate as u64) < expected {
                return Err(processing_error(
                    "Il file termina prima della durata dichiarata. Verifica che sia completo.",
                ));
            }
        }
        progress(100);
        Ok(())
    })();
    if let Err(error) = result {
        for chunk in &chunks {
            let _ = fs::remove_file(folder.join(&chunk.path));
        }
        return Err(error);
    }
    Ok(chunks)
}
fn commit_chunk(
    folder: &Path,
    prefix: &str,
    chunks: &mut Vec<AudioChunk>,
    samples: &[f32],
    rate: u32,
    start_frames: u64,
) -> AppResult<()> {
    let name = format!("{prefix}-{:06}.wav", chunks.len());
    let partial = folder.join(format!("{name}.part"));
    if let Err(error) = write_wav(&partial, samples, rate) {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    if let Err(error) = fs::rename(&partial, folder.join(&name)) {
        let _ = fs::remove_file(&partial);
        return Err(error.into());
    }
    chunks.push(AudioChunk {
        source: "import".into(),
        sequence: chunks.len() as u64,
        path: name,
        start_ms: start_frames * 1000 / rate as u64,
        end_ms: (start_frames + samples.len() as u64) * 1000 / rate as u64,
        sample_rate: rate,
        channels: 1,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_wav_in_bounded_timed_chunks() {
        let root =
            std::env::temp_dir().join(format!("qamille-import-test-{}", unique_id().unwrap()));
        fs::create_dir(&root).unwrap();
        let input = root.join("input.wav");
        let out = root.join("out");
        write_wav(&input, &vec![0.05; 16_000 * 6], 16_000).unwrap();
        let chunks = import_audio(&input, &out, &AtomicBool::new(false), &|_| {}).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!((chunks[0].start_ms, chunks[0].end_ms), (0, 5000));
        assert_eq!((chunks[1].start_ms, chunks[1].end_ms), (5000, 6000));
        let originals: Vec<_> = fs::read_dir(&out)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("original-")
            })
            .collect();
        assert_eq!(originals.len(), 1);
        assert_eq!(fs::read(&originals[0]).unwrap(), fs::read(&input).unwrap());
        fs::remove_file(&originals[0]).unwrap();
        for chunk in chunks {
            fs::remove_file(out.join(chunk.path)).unwrap();
        }
        fs::remove_dir(out).unwrap();
        fs::remove_file(input).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn cancelled_import_does_not_start_decoding() {
        let result = import_audio(
            Path::new("missing.wav"),
            Path::new("unused"),
            &AtomicBool::new(true),
            &|_| {},
        );
        assert_eq!(result.unwrap_err().code, "cancelled");
    }

    #[test]
    fn invalid_import_removes_snapshot_but_keeps_external_source() {
        let root =
            std::env::temp_dir().join(format!("qamille-invalid-import-{}", unique_id().unwrap()));
        fs::create_dir(&root).unwrap();
        let input = root.join("invalid.wav");
        let out = root.join("out");
        fs::write(&input, b"not an audio file").unwrap();
        assert!(import_audio(&input, &out, &AtomicBool::new(false), &|_| {}).is_err());
        assert_eq!(fs::read_dir(&out).unwrap().count(), 0);
        assert_eq!(fs::read(&input).unwrap(), b"not an audio file");
        fs::remove_dir(out).unwrap();
        fs::remove_file(input).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
