use clap::Parser;
use claxon::FlacReader;
use chrono::Local;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Dynamic Range meter for FLAC files.
/// Computes the DR value per the DR Loudness Standard (Pleasurize Music Foundation).
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Folder containing FLAC files (default: current directory)
    #[arg(default_value = ".")]
    folder: PathBuf,

    /// Output report file path (default: <folder>/dr_report.txt)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Suppress console output
    #[arg(short, long)]
    quiet: bool,
}

// ─── DR Algorithm ────────────────────────────────────────────────────────────
//
// Ported from https://codeberg.org/janw/drmeter/src/branch/main/drmeter/algorithm.py
//
//  1. Split each channel into non-overlapping blocks of round(3 * sample_rate) samples.
//  2. For each block compute:
//       • RMS  = sqrt( mean( 2 * |x|² ) )   ← note the factor of 2
//       • Peak = max( |x| )
//  3. Sort all blocks ascending by RMS and Peak independently.
//  4. top_n      = round( total_blocks * 0.2 )
//     rms_loud   = sqrt( sum( rms[-top_n:]² ) / top_n )
//     peak_loud  = peak[-2]   (2nd highest peak block, NTH_HIGHEST_PEAK = 2)
//  5. DR_channel = 20 * log10( peak_loud / rms_loud )  (0.0 if rms_loud == 0)
//  6. DR_track   = mean( DR_channel ), rounded to nearest integer.

const BLOCKSIZE_SECONDS: f64 = 3.0;
const UPMOST_BLOCKS_RATIO: f64 = 0.2;
const NTH_HIGHEST_PEAK: usize = 2; // 1-based from top → [-2] in Python

fn block_size_for_sample_rate(sample_rate: u32) -> usize {
    (BLOCKSIZE_SECONDS * sample_rate as f64).round() as usize
}

#[derive(Debug, Clone)]
struct BlockStats {
    rms: f64,
    peak: f64,
}

fn compute_block_stats(samples: &[f64]) -> BlockStats {
    let n = samples.len() as f64;
    // RMS: sqrt( mean( 2 * |x|² ) )
    let rms = (samples.iter().map(|x| 2.0 * x * x).sum::<f64>() / n).sqrt();
    let peak = samples.iter().cloned().fold(0.0f64, |a, x| a.max(x.abs()));
    BlockStats { rms, peak }
}

fn dr_for_channel(blocks: &[BlockStats]) -> f64 {
    if blocks.is_empty() {
        return 0.0;
    }

    let total = blocks.len();

    // Sort RMS values ascending (mirrors block_rms.sort(axis=0))
    let mut rms_sorted: Vec<f64> = blocks.iter().map(|b| b.rms).collect();
    rms_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    // Sort peak values ascending independently (mirrors block_peak.sort(axis=0))
    let mut peak_sorted: Vec<f64> = blocks.iter().map(|b| b.peak).collect();
    peak_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    // peak_loud = block_peak[-NTH_HIGHEST_PEAK] = 2nd highest
    let peak_idx = total.saturating_sub(NTH_HIGHEST_PEAK);
    let peak_loud = peak_sorted[peak_idx];

    // top 20% blocks by RMS: last top_n elements of the sorted array
    let top_n = ((total as f64 * UPMOST_BLOCKS_RATIO).round() as usize).max(1);
    let upmost_rms = &rms_sorted[(total - top_n)..];

    // rms_loud = sqrt( sum( rms² ) / top_n )
    let rms_loud = (upmost_rms.iter().map(|r| r * r).sum::<f64>() / top_n as f64).sqrt();

    if rms_loud <= 0.0 {
        return 0.0;
    }

    20.0 * (peak_loud / rms_loud).log10()
}

// ─── File processing ──────────────────────────────────────────────────────────

#[derive(Debug)]
struct TrackResult {
    filename: String,
    dr: i32,
    peak_db: f64,
    rms_db: f64,
    duration_secs: f64,
    channels: u32,
    sample_rate: u32,
    bit_depth: u32,
}

fn process_flac(path: &Path) -> Result<TrackResult, String> {
    let mut reader = FlacReader::open(path)
        .map_err(|e| format!("Cannot open: {}", e))?;

    let info = reader.streaminfo();
    let channels = info.channels;
    let sample_rate = info.sample_rate;
    let bits_per_sample = info.bits_per_sample;
    let total_samples = info.samples.unwrap_or(0);
    let duration_secs = if sample_rate > 0 {
        total_samples as f64 / sample_rate as f64
    } else {
        0.0
    };

    let scale = (1i64 << (bits_per_sample - 1)) as f64;
    let block_len = block_size_for_sample_rate(sample_rate);

    // Per-channel sample buffers
    let mut ch_buffers: Vec<Vec<f64>> = vec![Vec::new(); channels as usize];
    // Per-channel block stats
    let mut ch_blocks: Vec<Vec<BlockStats>> = vec![Vec::new(); channels as usize];

    // Interleaved sample iteration
    let mut samples_iter = reader.samples();

    loop {
        // Read one inter-channel frame
        let mut frame = Vec::with_capacity(channels as usize);
        let mut eof = false;
        for _ in 0..channels {
            match samples_iter.next() {
                Some(Ok(s)) => frame.push(s as f64 / scale),
                Some(Err(_)) => { eof = true; break; }
                None => { eof = true; break; }
            }
        }
        if frame.len() == channels as usize {
            for (ch, &s) in frame.iter().enumerate() {
                ch_buffers[ch].push(s);
            }
        }

        // Flush full blocks
        let buf_len = ch_buffers[0].len();
        if buf_len >= block_len || (eof && buf_len > 0) {
            let take = if buf_len >= block_len { block_len } else { buf_len };
            for ch in 0..channels as usize {
                let block: Vec<f64> = ch_buffers[ch].drain(..take).collect();
                ch_blocks[ch].push(compute_block_stats(&block));
            }
        }

        if eof { break; }
    }

    // Compute per-channel DR and aggregate
    let dr_values: Vec<f64> = (0..channels as usize)
        .map(|ch| dr_for_channel(&ch_blocks[ch]))
        .collect();

    let dr_mean = dr_values.iter().sum::<f64>() / dr_values.len() as f64;
    let dr = dr_mean.round() as i32;

    // Overall peak & RMS across all channels
    let all_blocks: Vec<&BlockStats> = ch_blocks.iter().flat_map(|v| v.iter()).collect();
    let overall_peak = all_blocks.iter().map(|b| b.peak).fold(0.0f64, f64::max);
    let overall_rms = {
        let sq: f64 = all_blocks.iter().map(|b| b.rms * b.rms).sum();
        (sq / all_blocks.len().max(1) as f64).sqrt()
    };

    fn to_db(linear: f64) -> f64 {
        if linear < 1e-10 { -100.0 } else { 20.0 * linear.log10() }
    }

    let filename = path.file_name().unwrap_or_default().to_string_lossy().into_owned();

    Ok(TrackResult {
        filename,
        dr,
        peak_db: to_db(overall_peak),
        rms_db: to_db(overall_rms),
        duration_secs,
        channels,
        sample_rate,
        bit_depth: bits_per_sample,
    })
}

// ─── Report formatting ────────────────────────────────────────────────────────

fn format_duration(secs: f64) -> String {
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

fn write_report(
    results: &[Result<TrackResult, (String, String)>],
    folder: &Path,
    output_path: &Path,
) -> std::io::Result<()> {
    let mut f = File::create(output_path)?;

    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let folder_str = folder.canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf())
        .display()
        .to_string();

    // Header
    writeln!(f, "═══════════════════════════════════════════════════════════════════════════")?;
    writeln!(f, "  Dynamic Range Report")?;
    writeln!(f, "  Generated : {}", timestamp)?;
    writeln!(f, "  Folder    : {}", folder_str)?;
    writeln!(f, "═══════════════════════════════════════════════════════════════════════════")?;
    writeln!(f)?;

    // Column headers
    writeln!(
        f,
        "  {:<4}  {:<8}  {:<8}  {:<8}  {:<8}  {}",
        "DR", "Peak dB", "RMS dB", "Duration", "Info", "File"
    )?;
    writeln!(f, "  {}", "─".repeat(73))?;

    let mut dr_values: Vec<i32> = Vec::new();
    let mut errors: Vec<(&str, &str)> = Vec::new();

    for result in results {
        match result {
            Ok(t) => {
                let info = format!(
                    "{}/{}/{}",
                    t.sample_rate / 1000,
                    t.bit_depth,
                    t.channels
                );
                writeln!(
                    f,
                    "  {:<4}  {:>+8.2}  {:>+8.2}  {:<8}  {:<8}  {}",
                    format!("DR{}", t.dr),
                    t.peak_db,
                    t.rms_db,
                    format_duration(t.duration_secs),
                    info,
                    t.filename
                )?;
                dr_values.push(t.dr);
            }
            Err((name, err)) => {
                errors.push((name, err));
            }
        }
    }

    writeln!(f, "  {}", "─".repeat(73))?;
    writeln!(f)?;

    // Summary
    if !dr_values.is_empty() {
        let dr_min = dr_values.iter().cloned().min().unwrap();
        let dr_max = dr_values.iter().cloned().max().unwrap();
        let dr_avg = dr_values.iter().sum::<i32>() as f64 / dr_values.len() as f64;
        let dr_album = dr_avg.round() as i32;

        writeln!(f, "  Summary")?;
        writeln!(f, "  ───────────────────────────────")?;
        writeln!(f, "  Tracks analysed : {}", dr_values.len())?;
        writeln!(f, "  Album DR        : DR{}", dr_album)?;
        writeln!(f, "  DR range        : DR{} – DR{}", dr_min, dr_max)?;
        writeln!(f)?;

        // Rating
        let rating = match dr_album {
            dr if dr >= 14 => "Excellent – wide dynamic range",
            dr if dr >= 10 => "Good",
            dr if dr >= 8  => "Acceptable",
            dr if dr >= 6  => "Compressed",
            _               => "Heavily brick-walled / clipped",
        };
        writeln!(f, "  DR Rating : {}", rating)?;
        writeln!(f)?;
    }

    // Errors
    if !errors.is_empty() {
        writeln!(f, "  Errors")?;
        writeln!(f, "  ───────────────────────────────")?;
        for (name, err) in &errors {
            writeln!(f, "  ✗ {} — {}", name, err)?;
        }
        writeln!(f)?;
    }

    writeln!(f, "═══════════════════════════════════════════════════════════════════════════")?;
    writeln!(f, "  DR Loudness Standard — https://www.dynamicrange.de")?;
    writeln!(f, "═══════════════════════════════════════════════════════════════════════════")?;

    Ok(())
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let args = Args::parse();

    let folder = &args.folder;
    if !folder.exists() || !folder.is_dir() {
        eprintln!("Error: '{}' is not a valid directory.", folder.display());
        std::process::exit(1);
    }

    // Collect FLAC files, sorted by name
    let mut flac_files: Vec<PathBuf> = fs::read_dir(folder)
        .expect("Cannot read directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .map(|ext| ext.eq_ignore_ascii_case("flac"))
                    .unwrap_or(false)
        })
        .collect();
    flac_files.sort();

    if flac_files.is_empty() {
        eprintln!("No FLAC files found in '{}'.", folder.display());
        std::process::exit(0);
    }

    if !args.quiet {
        println!("DR Measure — found {} FLAC file(s) in {}\n", flac_files.len(), folder.display());
    }

    let total = flac_files.len();
    let mut results: Vec<Result<TrackResult, (String, String)>> = Vec::with_capacity(total);

    for (i, path) in flac_files.iter().enumerate() {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if !args.quiet {
            print!("  [{}/{}] Analysing {} … ", i + 1, total, name);
            let _ = std::io::stdout().flush();
        }
        let t0 = Instant::now();
        match process_flac(path) {
            Ok(track) => {
                if !args.quiet {
                    println!("DR{} ({:.1}s)", track.dr, t0.elapsed().as_secs_f32());
                }
                results.push(Ok(track));
            }
            Err(e) => {
                if !args.quiet {
                    println!("ERROR: {}", e);
                }
                results.push(Err((name, e)));
            }
        }
    }

    // Determine output path
    let output_path = args.output.unwrap_or_else(|| folder.join("dr_report.txt"));

    match write_report(&results, folder, &output_path) {
        Ok(()) => {
            if !args.quiet {
                println!("\n  Report written → {}", output_path.display());
            }
        }
        Err(e) => {
            eprintln!("Failed to write report: {}", e);
            std::process::exit(1);
        }
    }
}


