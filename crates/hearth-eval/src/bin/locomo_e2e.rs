//! End-to-end LoCoMo QA: recall, answer with a real model, score with F1.
//!
//! This follows LoCoMo's own protocol rather than an LLM judge: the model is
//! asked for a short phrase, and the answer is scored with LoCoMo's
//! `normalize_answer` + Porter-stemmed F1. That makes these numbers directly
//! comparable to published LoCoMo accuracy.
//!
//! Cost: one model call per question. Use the limit argument for a pilot.
//!
//! Usage: `locomo_e2e <locomo10.json> [question-limit] [concurrency]`

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result};
use hearth_core::Harness;
use hearth_memory::{MemoryInput, MemoryService};
use hearth_model::{
    ChatMessage, ChatRequest, ContentBlock, OpenAiCompatibleProvider, Provider, Role, StreamEvent,
};
use rust_stemmers::{Algorithm, Stemmer};
use serde_json::Value;
use tempfile::TempDir;

/// LoCoMo's QA prompt, verbatim from `task_eval/gpt_utils.py`.
const QA_PROMPT: &str = "Based on the above context, write an answer in the form of a short phrase for the following question. Answer with exact words from the context whenever possible.\n\nQuestion: {} Short answer:\n";
/// LoCoMo's adversarial (category 5) variant: no "use exact words" nudge.
const QA_PROMPT_CAT5: &str =
    "Based on the above context, answer the following question.\n\nQuestion: {} Short answer:\n";

struct Turn {
    speaker: String,
    text: String,
    dia_id: String,
    session_date: String,
}

struct Question {
    question: String,
    /// LoCoMo gold answer rendered the way Python's `str()` would: integers
    /// keep their digits (2022 stays "2022") and null becomes "None". Three
    /// hundred-odd gold answers are integers, and coercing them to "" silently
    /// scored those questions wrong.
    answer: String,
    category: u8,
}

/// LoCoMo's `normalize_answer`: drop commas, articles, punctuation, case.
fn normalize_answer(text: &str) -> String {
    let lowered = text.to_lowercase().replace(',', "");
    let no_punct: String = lowered
        .chars()
        .filter(|character| !character.is_ascii_punctuation())
        .collect();
    let words: Vec<&str> = no_punct
        .split_whitespace()
        .filter(|word| !matches!(*word, "a" | "an" | "the" | "and"))
        .collect();
    words.join(" ")
}

/// LoCoMo's `f1_score`: Porter-stemmed token overlap.
fn f1_score(prediction: &str, ground_truth: &str) -> f64 {
    let stemmer = Stemmer::create(Algorithm::English);
    let stem = |text: &str| -> Vec<String> {
        normalize_answer(text)
            .split_whitespace()
            .map(|word| stemmer.stem(word).into_owned())
            .collect()
    };
    let predicted = stem(prediction);
    let truth = stem(ground_truth);
    if predicted.is_empty() || truth.is_empty() {
        return 0.0;
    }
    let truth_counts: BTreeMap<&String, usize> =
        truth.iter().fold(BTreeMap::new(), |mut map, word| {
            *map.entry(word).or_default() += 1;
            map
        });
    let mut remaining = truth_counts.clone();
    let mut common = 0usize;
    for word in &predicted {
        if let Some(count) = remaining.get_mut(word)
            && *count > 0
        {
            *count -= 1;
            common += 1;
        }
    }
    if common == 0 {
        return 0.0;
    }
    let precision = common as f64 / predicted.len() as f64;
    let recall = common as f64 / truth.len() as f64;
    if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    }
}

/// LoCoMo's `eval_question_answering` dispatch, exactly.
///
/// Categories 2, 3, 4 use plain `f1_score`; category 1 uses the comma-split
/// `f1`; category 5 is an abstention check, not an overlap score; category 3
/// gold answers are truncated at the first semicolon before scoring.
fn locomo_score(prediction: &str, gold: &str, category: u8) -> f64 {
    match category {
        2 | 4 => f1_score(prediction, gold),
        3 => f1_score(prediction, gold.split(';').next().unwrap_or(gold).trim()),
        1 => f1(prediction, gold),
        5 => {
            let lowered = prediction.to_lowercase();
            if lowered.contains("no information available") || lowered.contains("not mentioned") {
                1.0
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

/// LoCoMo's `f1`: split comma lists, best match per ground-truth variant.
fn f1(prediction: &str, ground_truth: &str) -> f64 {
    let predictions: Vec<&str> = prediction.split(',').map(str::trim).collect();
    let truths: Vec<&str> = ground_truth.split(',').map(str::trim).collect();
    let mut total = 0.0;
    for truth in &truths {
        let best = predictions
            .iter()
            .map(|prediction| f1_score(prediction, truth))
            .fold(0.0f64, f64::max);
        total += best;
    }
    total / truths.len().max(1) as f64
}

fn ask(
    provider: &dyn Provider,
    model: &str,
    context: &str,
    question: &str,
    category: u8,
) -> Result<String> {
    let template = if category == 5 {
        QA_PROMPT_CAT5
    } else {
        QA_PROMPT
    };
    let prompt = format!("{context}\n\n{}", template.replace("{}", question));
    let request = ChatRequest {
        model: model.to_owned(),
        messages: vec![ChatMessage {
            role: Role::User,
            blocks: vec![ContentBlock::Text(prompt)],
        }],
        tools: Vec::new(),
        temperature: Some(0.0),
        max_tokens: Some(600),
        session_id: None,
    };
    let stream = provider
        .stream(request)
        .map_err(|error| anyhow::anyhow!("provider: {error}"))?;
    let mut text = String::new();
    let mut thinking = String::new();
    for event in stream {
        match event {
            Ok(StreamEvent::TextDelta(delta)) => text.push_str(&delta),
            Ok(StreamEvent::ThinkingDelta(delta)) => thinking.push_str(&delta),
            Ok(_) => {}
            Err(error) => return Err(anyhow::anyhow!("stream: {error}")),
        }
    }
    if text.trim().is_empty() && std::env::var("BENCH_DEBUG").is_ok() {
        println!("   [empty answer, thinking={} chars]", thinking.len());
    }
    Ok(text.trim().to_owned())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .context("usage: locomo_e2e <data> [limit] [concurrency]")?;
    let limit: usize = args
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(usize::MAX);
    let workers: usize = args
        .get(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or(4);

    // Provider settings come from the configured home; the key is never printed.
    let home = std::env::var_os("HEARTH_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".hearth")))
        .context("HEARTH_HOME or HOME")?;
    let settings: Value = serde_json::from_str(
        &std::fs::read_to_string(home.join("settings.json")).context("settings")?,
    )?;
    let models = &settings["models"];
    let base_url = models["base_url"].as_str().context("base_url")?;
    let api_key = models["api_key"].as_str().context("api_key")?;
    let model = std::env::var("BENCH_MODEL")
        .ok()
        .or_else(|| models["model"].as_str().map(str::to_owned))
        .context("model")?;
    let headers: Vec<(String, String)> = models["headers"]
        .as_object()
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let provider: Arc<dyn Provider> = Arc::new(OpenAiCompatibleProvider::new(
        base_url,
        api_key,
        headers,
        models["session_header"].as_str().map(str::to_owned),
    )?);
    println!("model: {model}");

    let raw: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let conversations = raw.as_array().context("array")?;

    // Answers are cached so changing the scorer never costs another model call.
    let cache_path = PathBuf::from(
        std::env::var("BENCH_CACHE")
            .unwrap_or_else(|_| "/tmp/hearth-temp/locomo-e2e-cache.json".to_owned()),
    );
    let mut cache: BTreeMap<String, String> = std::fs::read_to_string(&cache_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    println!("cache: {} entries at {}", cache.len(), cache_path.display());

    let mut all_scores: Vec<(u8, f64, usize)> = Vec::new();
    let mut budget = limit;
    let started_all = Instant::now();

    for (index, conversation) in conversations.iter().enumerate() {
        if budget == 0 {
            break;
        }
        let (turns, questions) = parse_conversation(conversation)?;
        let sample = conversation["sample_id"].as_str().unwrap_or("?");
        println!(
            "\n== conversation {index} ({sample}) turns={} questions={}",
            turns.len(),
            questions.len()
        );

        let home = TempDir::new()?;
        let harness = Harness::open(home.path())?;
        let session = harness.session().clone();
        let memory = MemoryService::new(session)?;

        let ingest = Instant::now();
        let mut id_to_dia: BTreeMap<String, String> = BTreeMap::new();
        for turn in &turns {
            let (saved, _created) = memory.store().add(&MemoryInput {
                content: format!("{}: {}", turn.speaker, turn.text),
                tags: Some(vec![
                    format!("date {}", turn.session_date),
                    turn.speaker.clone(),
                ]),
                source: Some(turn.dia_id.clone()),
            })?;
            id_to_dia.insert(saved.id, turn.dia_id.clone());
        }
        println!("   ingest {:.1}s", ingest.elapsed().as_secs_f64());

        // Build every prompt for this conversation first, then answer in parallel.
        let mut jobs: Vec<(usize, String, String, u8, f64)> = Vec::new();
        for (position, question) in questions.iter().enumerate() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let recalled = memory.recall(&question.question)?;
            let lines: Vec<String> = recalled
                .iter()
                .filter_map(|item| id_to_dia.get(&item.id))
                .filter_map(|dia| {
                    turns.iter().find(|turn| &turn.dia_id == dia).map(|turn| {
                        format!("- {} ({}): {}", turn.session_date, turn.speaker, turn.text)
                    })
                })
                .collect();
            let context = format!(
                "Below is what you remember from a long conversation.\n\n{}",
                lines.join("\n")
            );
            jobs.push((
                position,
                context,
                question.question.clone(),
                question.category,
                f64::NAN,
            ));
        }

        // Seed answers from the cache; only the rest cost a model call.
        let mut seeded: Vec<String> = vec![String::new(); jobs.len()];
        let mut pending = 0usize;
        for (slot, (position, _, _, _, _)) in jobs.iter().enumerate() {
            let key = format!("{sample}:{position}");
            if let Some(text) = cache.get(&key) {
                seeded[slot] = text.clone();
            } else {
                pending += 1;
            }
        }
        println!(
            "   cached {} of {} answers",
            jobs.len() - pending,
            jobs.len()
        );

        let next = Arc::new(AtomicUsize::new(0));
        let answers = Arc::new(std::sync::Mutex::new(seeded));
        let failures = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..workers.min(jobs.len().max(1)) {
            let provider = Arc::clone(&provider);
            let model = model.clone();
            let jobs = jobs.clone();
            let next = Arc::clone(&next);
            let answers = Arc::clone(&answers);
            let failures = Arc::clone(&failures);
            handles.push(std::thread::spawn(move || {
                loop {
                    let position = next.fetch_add(1, Ordering::Relaxed);
                    if position >= jobs.len() {
                        break;
                    }
                    // Already cached: nothing to spend.
                    if answers
                        .lock()
                        .map(|guard| !guard[position].is_empty())
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    let (_, context, question, category, _) = &jobs[position];
                    match ask(provider.as_ref(), &model, context, question, *category) {
                        Ok(text) => {
                            if let Ok(mut guard) = answers.lock() {
                                guard[position] = text;
                            }
                        }
                        Err(_) => {
                            failures.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }));
        }
        for handle in handles {
            let _ = handle.join();
        }
        let answers = answers.lock().expect("answers").clone();
        let failed = failures.load(Ordering::Relaxed);
        for (slot, (position, _, _, _, _)) in jobs.iter().enumerate() {
            if !answers[slot].is_empty() {
                cache.insert(format!("{sample}:{position}"), answers[slot].clone());
            }
        }
        if let Ok(text) = serde_json::to_string_pretty(&cache) {
            let _ = std::fs::write(&cache_path, text);
        }

        let mut local: BTreeMap<u8, Vec<f64>> = BTreeMap::new();
        for (slot, (position, _, _, category, _)) in jobs.iter().enumerate() {
            let Some(question) = questions.get(*position) else {
                continue;
            };
            let score = locomo_score(&answers[slot], &question.answer, question.category);
            if std::env::var("BENCH_DEBUG").is_ok() {
                println!(
                    "   Q: {}\n   gold: {}\n   pred: {}\n   F1: {score:.3}",
                    question.question, question.answer, answers[slot]
                );
            }
            local.entry(*category).or_default().push(score);
            all_scores.push((*category, score, *position));
        }
        let mean: f64 = local.values().flatten().sum::<f64>()
            / local.values().map(Vec::len).sum::<usize>().max(1) as f64;
        println!(
            "   answered {} (failed {failed}) mean F1 {:.3}",
            jobs.len(),
            mean
        );
    }

    println!("\n== end-to-end LoCoMo QA (F1) ==");
    let mut by_category: BTreeMap<u8, (usize, f64)> = BTreeMap::new();
    for (category, score, _) in &all_scores {
        let entry = by_category.entry(*category).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += score;
    }
    let names = [
        (1u8, "multi-hop"),
        (2, "temporal"),
        (3, "open-domain"),
        (4, "single-hop"),
        (5, "adversarial"),
    ];
    for (category, name) in names {
        if let Some((count, sum)) = by_category.get(&category) {
            println!("  {name:12} n={count:<4} F1={:.3}", sum / *count as f64);
        }
    }
    let total: f64 = all_scores.iter().map(|(_, score, _)| score).sum();
    let count = all_scores.len().max(1);
    println!(
        "  {:12} n={:<4} F1={:.3}",
        "OVERALL",
        all_scores.len(),
        total / count as f64
    );
    println!(
        "\ntotal {:.1} minutes, {} questions",
        started_all.elapsed().as_secs_f64() / 60.0,
        all_scores.len()
    );
    Ok(())
}

fn parse_conversation(conversation: &Value) -> Result<(Vec<Turn>, Vec<Question>)> {
    let dialog = conversation["conversation"]
        .as_object()
        .context("conversation")?;
    let mut sessions: Vec<usize> = Vec::new();
    for key in dialog.keys() {
        if let Some(rest) = key.strip_prefix("session_")
            && let Ok(number) = rest.parse::<usize>()
        {
            sessions.push(number);
        }
    }
    sessions.sort_unstable();
    sessions.dedup();

    let mut turns = Vec::new();
    for session in sessions {
        let date = dialog
            .get(&format!("session_{session}_date_time"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let entries = dialog
            .get(&format!("session_{session}"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for entry in entries {
            let text = entry["text"].as_str().unwrap_or_default().trim();
            if text.is_empty() {
                continue;
            }
            turns.push(Turn {
                speaker: entry["speaker"].as_str().unwrap_or("someone").to_owned(),
                text: text.to_owned(),
                dia_id: entry["dia_id"].as_str().unwrap_or_default().to_owned(),
                session_date: date.clone(),
            });
        }
    }

    let mut questions = Vec::new();
    for entry in conversation["qa"].as_array().cloned().unwrap_or_default() {
        // Gold answers are rendered the way Python's str() would: an integer
        // keeps its digits and null becomes "None". Several hundred gold answers
        // are integers, and coercing them to "" scored those questions wrong.
        let answer = match &entry["answer"] {
            Value::String(text) => text.clone(),
            Value::Null => "None".to_owned(),
            other => other.to_string(),
        };
        questions.push(Question {
            question: entry["question"].as_str().unwrap_or_default().to_owned(),
            answer,
            category: entry["category"].as_u64().unwrap_or(0) as u8,
        });
    }
    Ok((turns, questions))
}
