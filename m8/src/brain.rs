//! The mate's brain, the same for every game. Everything goes into one feed: what the player
//! and the mate say, and what happens in the game. The LLM and Kev get the exact same prompt:
//! the system prompt, the level's story so far, what the mate remembers, the feed, and, before
//! its newest lines, what holds right now (the combat or radio prompt while the game says so,
//! and its current state).
//!
//! OpenCE owns ordinary movement, recovery and combat. Player utterances get one
//! focused intent question; game events only ask whether Kevin should speak.
//!
//! Memory: when the feed gets long, the LLM folds its oldest lines into a short summary. The
//! game's saves carry the memory: a save stores it, and loading that save brings it back (dying
//! and going back to a checkpoint forgets what happened since).

use crate::config::Config;
use crate::link::GameLink;
use crate::picture;
use crate::site::{PromptFile, Trace};
use crate::speak::SpeechJob;
use crate::Speaking;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

pub enum Input {
    #[cfg(test)]
    Stop,
    /// Something the player said (mic, console or the site).
    Heard {
        text: String,
        context: Option<u64>,
    },
    /// Something that happened in the game. `react` asks Clef whether to speak;
    /// otherwise it only goes into the feed (e.g. dialogue in a cutscene).
    Event {
        text: String,
        react: bool,
        permit: Option<EventPermit>,
    },
    /// The game saved (its id for the save), or loaded a save.
    Saved(String),
    /// Native diagnostics for history only; never conversational telemetry.
    Notice(String),
    Loaded(String),
    /// A summary of the feed's oldest lines is ready.
    Summary {
        memory: u64,
        folded: usize,
        text: String,
    },
}

/// Bound only low-priority events; player and lifecycle inputs keep the FIFO channel.
#[derive(Clone, Default)]
pub struct EventBudget(Arc<EventCounts>);
#[derive(Default)]
struct EventCounts {
    pending: AtomicUsize,
    dropped: AtomicUsize,
}
pub struct EventPermit(Arc<EventCounts>);
impl Drop for EventPermit {
    fn drop(&mut self) {
        self.0.pending.fetch_sub(1, Ordering::Relaxed);
    }
}
impl EventBudget {
    pub fn send(&self, inputs: &Sender<Input>, text: String, react: bool) {
        if self
            .0
            .pending
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |pending| {
                (pending < 64).then_some(pending + 1)
            })
            .is_err()
        {
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let _ = inputs.send(Input::Event {
            text,
            react,
            permit: Some(EventPermit(self.0.clone())),
        });
    }
}

impl Input {
    /// Capture before queueing: slow dialogue/classification must not move the
    /// destination to whatever Chief happens to aim at later.
    pub fn heard(text: String, link: &GameLink) -> Self {
        Self::Heard {
            text,
            context: link.capture_command(),
        }
    }
}

/// The feed's size, in tokens (counted as 4 characters each): over FEED_TOKENS, at a safe point
/// (the game saved, or it's been quiet), its oldest lines are folded into the summary until it's
/// down to FEED_FOLDED_TO; over FEED_MAX_TOKENS, whenever (the servers take 32k a request).
const FEED_TOKENS: usize = 16_000;
const FEED_FOLDED_TO: usize = 12_000;
const FEED_MAX_TOKENS: usize = 20_000;
/// Nothing from the game or the player, the mate not talking and no fight, this long: it's been
/// quiet, an event of its own.
const QUIET: Duration = Duration::from_secs(45);
const QUIET_EVENT: &str = "It's been quiet for a while.";
/// How many of the feed's newest lines Kev reads (the LLM reads it all): its server reads its
/// whole prompt afresh each call, so the longer it is, the slower every decision.
const KEV_FEED_LINES: usize = 20;
/// The saves remembered (older ones are forgotten).
const SAVES: usize = 50;

/// What the mate remembers: a summary of the past and the feed since.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Memory {
    summary: String,
    feed: Vec<String>,
}

pub struct Brain {
    cfg: Config,
    link: GameLink,
    speech: Sender<SpeechJob>,
    speaking: Speaking,
    agent: ureq::Agent,
    kev_agent: ureq::Agent,
    trace: Trace,
    system: PromptFile,
    combat: PromptFile,
    radio: PromptFile,
    /// (for summaries made on another thread)
    inputs: Sender<Input>,
    memory: Memory,
    /// Counts loads, so that a summary of a memory since replaced is dropped.
    memory_generation: u64,
    summarizing: bool,
    saves: Vec<(String, Memory)>,
}

impl Brain {
    pub fn new(
        cfg: Config,
        link: GameLink,
        speech: Sender<SpeechJob>,
        speaking: Speaking,
        trace: Trace,
        system: PromptFile,
        combat: PromptFile,
        radio: PromptFile,
        inputs: Sender<Input>,
    ) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(60))
            .build();
        let kev_agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_millis(cfg.kev.timeout_ms))
            .timeout_connect(Duration::from_millis(cfg.kev.timeout_ms.min(1000)))
            .build();
        let saves = std::fs::read_to_string(Self::saves_path(&cfg))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Brain {
            cfg,
            link,
            speech,
            speaking,
            agent,
            kev_agent,
            trace,
            system,
            combat,
            radio,
            inputs,
            memory: Memory::default(),
            memory_generation: 0,
            summarizing: false,
            saves,
        }
    }

    pub fn run(mut self, inputs: Receiver<Input>) {
        // (an input taken while gathering events, for next)
        let mut waiting = None;
        let mut quiet_since = Instant::now();
        loop {
            // (a quiet spell: a safe point to fold the feed)
            let mut quiet = false;
            let input = match waiting.take() {
                Some(input) => input,
                None => match inputs.recv_timeout(Duration::from_secs(1)) {
                    Ok(input) => input,
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {
                        let s = self.state();
                        if self.speaking.is_speaking()
                            || s.is_null()
                            || s["in_combat"].as_bool().unwrap_or(false)
                        {
                            quiet_since = Instant::now();
                        }
                        if quiet_since.elapsed() < QUIET {
                            continue;
                        }
                        quiet = true;
                        Input::Event {
                            text: QUIET_EVENT.to_string(),
                            react: true,
                            permit: None,
                        }
                    }
                },
            };
            // (anything from the game or the player breaks the quiet; a summary is the brain's own)
            if !matches!(input, Input::Summary { .. }) {
                quiet_since = Instant::now();
            }
            let result = match input {
                #[cfg(test)]
                Input::Stop => break,
                Input::Heard { text, context } => self.on_heard(&text, context),
                Input::Event {
                    text,
                    react,
                    permit,
                } => {
                    // (and the events that came in meanwhile, all at once: he reacts to what's
                    // happened since, not to each in turn, falling behind)
                    let mut permits: Vec<_> = permit.into_iter().collect();
                    let mut events = vec![(text, react)];
                    while let Ok(more) = inputs.try_recv() {
                        match more {
                            Input::Event {
                                text,
                                react,
                                permit,
                            } => {
                                permits.extend(permit);
                                events.push((text, react));
                            }
                            other => {
                                waiting = Some(other);
                                break;
                            }
                        }
                    }
                    if let Some(permit) = permits.first() {
                        let dropped = permit.0.dropped.swap(0, Ordering::Relaxed);
                        if dropped > 0 {
                            self.trace.add("input", json!({"events_dropped": dropped}));
                        }
                    }
                    self.on_events(&events)
                }
                Input::Saved(id) => self.on_saved(id),
                Input::Notice(text) => {
                    self.trace.add("companion", json!({"text": text}));
                    Ok(())
                }
                Input::Loaded(id) => self.on_loaded(&id),
                Input::Summary {
                    memory,
                    folded,
                    text,
                } => self.on_summary(memory, folded, text),
            };
            if let Err(e) = result {
                eprintln!("[brain] error: {e:#}");
            }
            self.fold_over(if quiet { FEED_TOKENS } else { FEED_MAX_TOKENS });
        }
    }

    fn on_heard(&mut self, text: &str, context: Option<u64>) -> Result<()> {
        self.link.send_json(&json!({"type": "heard", "text": text}));
        let player = self.player();
        self.memory.feed.push(format!("{player}: {text}"));
        self.trace.add("heard", json!({"text": text}));
        // A command failure must not take down the ordinary conversation.
        if let Err(error) = self.interpret_command(text, context) {
            self.trace.add(
                "command",
                json!({"text": text, "error": format!("{error:#}")}),
            );
            self.memory
                .feed
                .push("(Command interpretation failed; no player order was sent.)".into());
        }
        self.respond(true)
    }

    fn on_events(&mut self, events: &[(String, bool)]) -> Result<()> {
        for (text, react) in events {
            let text = self.fill(text);
            self.memory.feed.push(text.clone());
            // (and the game's state as it was then: what Kev and the LLM saw of it)
            self.trace.add(
                "event",
                json!({"text": text, "react": react, "state": self.state()}),
            );
        }
        if events.iter().any(|(_, react)| *react) {
            self.respond(false)?;
        }
        Ok(())
    }

    /// Player speech always gets dialogue; game events use Clef's speech
    /// relevance decision and the existing voice cooldown/interruption path.
    fn respond(&mut self, to_player: bool) -> Result<()> {
        let cooldown = !to_player
            && self.speaking.quiet_from().map_or(true, |quiet| {
                Instant::now() < quiet + self.link.speech_cooldown()
            });
        if self.decide(to_player, cooldown)? {
            // (still talking: he drops what he hadn't said yet; what's playing goes on until the
            // new line's first audio takes its place, speak.rs)
            if self.speaking.is_speaking() {
                self.speaking.interrupt();
            }
            self.say()?;
        }
        Ok(())
    }

    fn saves_path(cfg: &Config) -> PathBuf {
        cfg.base_dir.join("saves.json")
    }

    fn on_saved(&mut self, id: String) -> Result<()> {
        self.saves.retain(|(saved, _)| *saved != id);
        self.saves.push((id.clone(), self.memory.clone()));
        if self.saves.len() > SAVES {
            self.saves.remove(0);
        }
        std::fs::write(
            Self::saves_path(&self.cfg),
            serde_json::to_string(&self.saves)?,
        )?;
        self.trace
            .add("event", json!({"text": format!("(the game saved: {id})")}));
        // (a safe point to fold the feed)
        self.fold_over(FEED_TOKENS);
        Ok(())
    }

    /// Back to what the mate remembered at that save; nothing, for a save it never saw.
    fn on_loaded(&mut self, id: &str) -> Result<()> {
        let found = self.saves.iter().rev().find(|(saved, _)| saved == id);
        self.memory = found.map(|(_, memory)| memory.clone()).unwrap_or_default();
        self.memory_generation += 1;
        self.summarizing = false;
        if self.speaking.is_speaking() {
            self.speaking.interrupt();
            self.link.send_json(&json!({"type": "stop_audio"}));
        }
        let note = if found.is_some() {
            "memory restored"
        } else {
            "a save it never saw: memory cleared"
        };
        self.trace.add(
            "event",
            json!({"text": format!("(the game loaded {id}: {note})")}),
        );
        println!("[brain] loaded {id}: {note}");
        Ok(())
    }

    fn on_summary(&mut self, memory: u64, folded: usize, text: String) -> Result<()> {
        self.summarizing = false;
        if memory == self.memory_generation && folded <= self.memory.feed.len() {
            self.memory.feed.drain(..folded);
            self.memory.summary = text;
            self.trace.add(
                "memory",
                json!({"summary": self.memory.summary, "folded": folded}),
            );
        }
        Ok(())
    }

    /// The feed over `limit` tokens: its oldest lines, down to FEED_FOLDED_TO, are folded into the
    /// summary, on another thread (the LLM writes it), so the mate goes on meanwhile.
    fn fold_over(&mut self, limit: usize) {
        let tokens = |lines: &[String]| lines.iter().map(|line| line.len() + 1).sum::<usize>() / 4;
        if self.summarizing || tokens(&self.memory.feed) <= limit {
            return;
        }
        let mut folded = 0;
        while folded < self.memory.feed.len()
            && tokens(&self.memory.feed[folded..]) > FEED_FOLDED_TO
        {
            folded += 1;
        }
        self.summarizing = true;
        let (agent, url, name) = (
            self.agent.clone(),
            self.cfg.llm.url.clone(),
            self.cfg.mate.name.clone(),
        );
        let model = self.cfg.llm.model.clone();
        let (previous, lines) = (
            self.memory.summary.clone(),
            self.memory.feed[..folded].join("\n"),
        );
        let (inputs, memory, trace) = (
            self.inputs.clone(),
            self.memory_generation,
            self.trace.clone(),
        );
        let temperature = 0.3;
        std::thread::spawn(move || {
            let start = Instant::now();
            let request = format!(
                "{previous}\n{lines}\n\nRewrite all of the above as {name}'s memory of what happened: one paragraph of \
                 at most 250 words, past tense, about {name} in the third person. Keep every name, number and fact exactly \
                 as said (people, pets, places, plans, promises), and what matters for the story and the fights. Add \
                 nothing that isn't above. Leave out small talk that doesn't matter."
            );
            let messages = json!([{"role": "user", "content": request}]);
            let reply: Result<Value> = agent
                .post(&url)
                .send_json(
                    json!({"model": model, "messages": messages, "temperature": temperature}),
                )
                .map_err(anyhow::Error::from)
                .and_then(|r| Ok(r.into_json()?));
            let output = reply
                .as_ref()
                .ok()
                .and_then(|r| r["choices"][0]["message"]["content"].as_str())
                .map(str::to_string);
            trace.add(
                "summary",
                json!({
                    "messages": messages,
                    "settings": {"url": url, "temperature": temperature,
                                 "model": reply.as_ref().ok().and_then(|r| r["model"].as_str().map(str::to_string))},
                    "output": output,
                    "ms": start.elapsed().as_millis() as u64,
                    "error": reply.as_ref().err().map(|e| format!("{e:#}")),
                }),
            );
            match output {
                Some(output) => {
                    let _ = inputs.send(Input::Summary {
                        memory,
                        folded,
                        text: clean(&output),
                    });
                }
                None => eprintln!(
                    "[brain] summary failed: {:#}",
                    reply.err().map(|e| e.to_string()).unwrap_or_default()
                ),
            }
        });
    }

    fn state(&self) -> Value {
        self.link.state.lock().unwrap().clone()
    }

    /// The player's name: player_name.txt's (beside the config), else the game's.
    fn player(&self) -> String {
        std::fs::read_to_string(self.cfg.base_dir.join("player_name.txt"))
            .ok()
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .or_else(|| self.state()["player_name"].as_str().map(str::to_string))
            .unwrap_or_else(|| "the player".to_string())
    }

    /// The game's text (and the prompts) with the player's name and the game's in place of
    /// {player} and {game}.
    fn fill(&self, text: &str) -> String {
        text.replace("{player}", &self.player())
            .replace("{game}", self.state()["game"].as_str().unwrap_or("a game"))
    }

    /// The level's story so far, as the player knows it (levels/<level>.txt), and what the mate
    /// remembers of before the feed.
    fn context(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if let Some(story) = self.state()["level"].as_str().and_then(|level| {
            std::fs::read_to_string(
                self.cfg
                    .persona_dir()
                    .join("levels")
                    .join(format!("{level}.txt")),
            )
            .ok()
        }) {
            parts.push(self.fill(story.trim()));
        }
        if !self.memory.summary.is_empty() {
            parts.push(format!(
                "What {} remembers: {}",
                self.cfg.mate.name, self.memory.summary
            ));
        }
        parts
    }

    fn right_now(&self) -> String {
        format!(
            "Right now: {}",
            self.fill(self.state()["state"].as_str().unwrap_or("unknown"))
        )
    }

    /// Who the mate is: the system prompt.
    fn system(&self) -> String {
        self.fill(self.system.get().trim())
    }

    /// What holds right now, in both prompts just before the newest lines: the combat and radio
    /// prompts while the game says so, and the game's state. It changes all the time; before it,
    /// the prompt only grows, so the servers reuse what they read of it last time.
    fn situation(&self) -> String {
        let s = self.state();
        let mut parts = Vec::new();
        for (on, prompt) in [("in_combat", &self.combat), ("on_radio", &self.radio)] {
            let text = prompt.get();
            if s[on].as_bool().unwrap_or(false) && !text.trim().is_empty() {
                parts.push(self.fill(text.trim()));
            }
        }
        parts.push(self.right_now());
        parts.join("\n")
    }

    /// Where the feed's newest lines start: after the mate's own last line (the game's and the
    /// player's since).
    fn newest(&self) -> usize {
        let own = format!("{}: ", self.cfg.mate.name);
        self.memory
            .feed
            .iter()
            .rposition(|line| line.starts_with(&own))
            .map_or(0, |at| at + 1)
    }

    /// Kev's prompt: who the mate is, the context, the feed's newest KEV_FEED_LINES, with what
    /// holds right now before the lines since the mate last spoke, so that the newest line is
    /// last: what the LLM gets, as one block.
    fn prompt(&self) -> String {
        let start = self.memory.feed.len().saturating_sub(KEV_FEED_LINES);
        let newest = self.newest().max(start);
        let mut parts = vec![self.system()];
        parts.extend(self.context());
        if newest > start {
            parts.push(self.memory.feed[start..newest].join("\n"));
        }
        parts.push(self.situation());
        if newest < self.memory.feed.len() {
            parts.push(self.memory.feed[newest..].join("\n"));
        }
        parts.join("\n\n")
    }

    /// The LLM's: the same, as chat turns, which the server puts in the loaded model's own
    /// template. The system turn is who the mate is; then the context, the game's lines and the
    /// player's as the user's turns and the mate's own lines as its turns (a line or more in a
    /// row, one turn), what holds right now just before the newest lines. As one block of text,
    /// the model wrote a scene to carry on (a line, then another, each with its own voice) rather
    /// than the mate's turn.
    fn messages(&self) -> Vec<Value> {
        let mut turns: Vec<(&str, String)> = vec![("system", self.system())];
        let mut add = |role: &'static str, text: &str| match turns.last_mut() {
            Some((last, joined)) if *last == role => {
                joined.push('\n');
                joined.push_str(text);
            }
            _ => turns.push((role, text.to_string())),
        };
        let context = self.context().join("\n\n");
        if !context.is_empty() {
            add("user", &context);
        }
        let own = format!("{}: ", self.cfg.mate.name);
        let newest = self.newest();
        for line in &self.memory.feed[..newest] {
            match line.strip_prefix(&own) {
                Some(said) => add("assistant", said),
                None => add("user", line),
            }
        }
        add("user", &self.situation());
        for line in &self.memory.feed[newest..] {
            add("user", line);
        }
        let messages: Vec<Value> = turns
            .iter()
            .map(|(role, text)| json!({"role": role, "content": text}))
            .collect();
        self.trace.set_prompt(
            &turns
                .iter()
                .map(|(role, text)| format!("[{role}]\n{text}"))
                .collect::<Vec<_>>()
                .join("\n\n"),
        );
        messages
    }

    /// Exactly one intent choice for the current utterance. The general scene
    /// prompt is deliberately excluded: earlier lines and events are not orders.
    fn interpret_command(&mut self, text: &str, context: Option<u64>) -> Result<()> {
        let started = Instant::now();
        let mut calls = Vec::new();
        let question = json!({"intent": {
            "type": "choice",
            "instructions": "Interpret ONLY the player's current utterance. Identify a genuine, unambiguous instruction to change Kevin's gameplay behavior, including natural indirect requests. Questions asking Kevin to do something can be instructions; questions about what he is doing are conversation. Choose none for ordinary conversation, descriptions, jokes, quoted or hypothetical instructions, unsupported requests, ambiguous or conflicting instructions, or unclear destinations. Negative instructions such as do not shoot can mean hold_fire; never invert a negated movement request into a positive order. A location command must clearly mean the ground Chief was aiming at when speaking. Never infer an order from game events or prior dialogue.",
            "criteria": {
                "none": "No clear supported instruction; preserve every current order.",
                "follow": "Asks Kevin to accompany, return to, or follow Chief.",
                "stay": "Asks Kevin to wait, stay or hold his current position.",
                "go_aim": "Explicitly tells Kevin to move to the ground Chief is pointing or aiming at, such as go there.",
                "hold_fire": "Explicitly tells Kevin to stop shooting or hold fire.",
                "resume_fire": "Explicitly tells Kevin to resume shooting or fire at will."
            }
        }});
        let prompt = format!("Current player utterance: {}", serde_json::to_string(text)?);
        let answer = self.kev("intent", &prompt, &question, &mut calls);
        if let Err(error) = &answer {
            self.trace.add("kev", json!({"calls": calls, "action": "none", "sent": false,
                "purpose": "intent", "error": format!("{error:#}"), "ms": started.elapsed().as_millis() as u64}));
        }
        let answer = answer?;
        let intent = command_intent(&answer);
        self.trace.add("kev", json!({"calls": calls, "action": intent, "sent": false, "purpose": "intent", "ms": started.elapsed().as_millis() as u64}));
        if intent == "none" {
            return Ok(());
        }
        let result = context.and_then(|id| {
            self.link.request(
                json!({"type": "action", "action": intent, "context_id": id}),
                Duration::from_secs(1),
            )
        });
        let outcome = match result.as_ref() {
            Some(reply) if reply["accepted"].as_bool() == Some(true) => format!(
                "Kevin is now {}, with autonomous fire {}.",
                match reply["movement_order"].as_str() {
                    Some("follow") => "following Chief",
                    Some("stay") => "holding position",
                    Some("go_aim") => "moving to the requested location",
                    _ => "keeping his current movement order",
                },
                if reply["fire_enabled"].as_bool() == Some(true) { "enabled" } else { "disabled" },
            ),
            Some(reply) => format!("Order rejected; previous orders retained. {}",
                reply["text"].as_str().unwrap_or("The game could not validate this order.")),
            None if context.is_none() => "No order was sent because the game was unavailable when you spoke; please repeat the order.".into(),
            None => "The order was sent but the game has not confirmed it; do not claim it succeeded.".into(),
        };
        self.trace.add(
            "command",
            json!({"intent": intent, "context_id": context, "result": result, "text": outcome}),
        );
        self.memory.feed.push(format!("({outcome})"));
        Ok(())
    }

    /// Events only ask about speech, with the established cooldown. Player
    /// conversation always gets an answer; its intent was handled separately.
    fn decide(&mut self, to_player: bool, cooldown: bool) -> Result<bool> {
        if to_player {
            return Ok(true);
        }
        let start = Instant::now();
        let mut calls = Vec::new();
        let ask_speak = !to_player && !cooldown;
        let speak = to_player
            || ask_speak
                && {
                    let question = json!({"speak": {"type": "noul", "instructions": format!("Should {} speak now?", self.cfg.mate.name)}});
                    match self.kev("speak", &self.prompt(), &question, &mut calls) {
                        Ok(answer) => answer["speak"]["noul"].as_f64().unwrap_or(0.0) >= 0.5,
                        Err(error) => {
                            eprintln!("[brain] optional speech decision failed: {error:#}");
                            false
                        }
                    }
                };
        self.trace.add(
            "kev",
            json!({"calls": calls, "action": "none", "sent": false,
            "speak": speak, "speak_asked": ask_speak, "cooldown": cooldown,
            "ms": start.elapsed().as_millis() as u64}),
        );
        Ok(speak)
    }

    /// Kev picks one of `actions` (or nothing), and for one with targets, which. It goes to the
    /// game and in the feed, with what it found (its "result"), or, for one of the player's view
    /// (its "view"), what the mate makes of that. What was done: its `action`, `option`, `seen`.
    #[allow(dead_code)] // Legacy special actions are deliberately outside the active decision path.
    fn act(&mut self, purpose: &str, actions: &[Value], calls: &mut Vec<Value>) -> Result<Value> {
        let name = self.cfg.mate.name.clone();
        let mut options = serde_json::Map::new();
        options.insert(
            "nothing".to_string(),
            json!(format!("{name} carries on as he is")),
        );
        for a in actions {
            let description = self.fill(a["description"].as_str().unwrap_or_default());
            options.insert(
                a["name"].as_str().unwrap_or_default().to_string(),
                json!(format!("{name} {description}")),
            );
        }
        let prompt = self.prompt();
        let question = json!({"do": {"type": "choice", "instructions": format!("Which of these does {name} do now?"), "criteria": options}});
        let answers = self.kev(purpose, &prompt, &question, calls)?;
        let action = answers["do"]["choice"]
            .as_str()
            .unwrap_or("nothing")
            .to_string();
        if action == "nothing" {
            return Ok(json!({"action": action}));
        }
        let chosen = actions
            .iter()
            .find(|a| a["name"] == action.as_str())
            .cloned()
            .unwrap_or_default();
        let mut msg = json!({"type": "action", "action": action});
        // ("Kevin stands up." / "Kevin shoots at an enemy: grunt close (8 m) ahead.")
        let mut line = options[action.as_str()]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if let Some(targets) = chosen["options"].as_object().filter(|o| !o.is_empty()) {
            let which = json!({"which": {"type": "choice", "instructions": format!("{line}. Which one?"), "criteria": targets}});
            let which_purpose = if purpose == "do" {
                "which".to_string()
            } else {
                format!("{purpose}: which")
            };
            let answer = self.kev(&which_purpose, &prompt, &which, calls)?;
            msg["option"] = answer["which"]["choice"].clone();
            if let Some(label) = msg["option"]
                .as_str()
                .and_then(|id| targets.get(id))
                .and_then(Value::as_str)
            {
                line = format!("{line}: {label}");
            }
        }
        let mut seen = Value::Null;
        match chosen["view"].as_str() {
            Some(view) => {
                let (what, picture) = self.look(view, &mut msg)?;
                line = format!("{line}: {what}");
                seen = json!({"what": what, "picture": picture});
            }
            None => {
                self.link.send_json(&msg);
                if let Some(result) = chosen["result"].as_str() {
                    line = format!("{line}: {}", self.fill(result));
                }
            }
        }
        self.memory.feed.push(format!("{line}."));
        Ok(json!({"action": action, "option": msg["option"], "seen": seen}))
    }

    /// The mate's eyes: the player's view from the game, and what the LLM makes of it, as his
    /// eyes, not as him (what's there, plainly, not a line of his), from the game's state and the
    /// feed's newest lines. "describe": what's there; "choose": which of the spots marked on it
    /// he goes to (sent to the game as the action's option). What goes in the feed, and the
    /// picture's name (logs/views/<name>.png, kept for the history page).
    fn look(&mut self, kind: &str, msg: &mut Value) -> Result<(String, Value)> {
        let (name, player) = (self.cfg.mate.name.clone(), self.player());
        let start = Instant::now();
        let Some(mut view) = self.link.view(Duration::from_secs(2)) else {
            self.trace.add(
                "sight",
                json!({"kind": kind, "error": "the game sent no view"}),
            );
            return Ok(("he can't see it".to_string(), Value::Null));
        };
        let choose = kind == "choose";
        if choose {
            picture::mark(&mut view.rgb, view.width, view.height, &view.spots);
        }
        let png = picture::png(&view.rgb, view.width, view.height);
        let file = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .to_string();
        let views = self.cfg.base_dir.join("logs").join("views");
        let _ = std::fs::create_dir_all(&views);
        let _ = std::fs::write(views.join(format!("{file}.png")), &png);

        let game = self.state()["game"]
            .as_str()
            .unwrap_or("a game")
            .to_string();
        let eyes = format!("You're the eyes of {name}, {player}'s teammate in {game}: you're shown what {player} is looking at.");
        let (system, question) = if choose {
            (
                format!("{eyes} Numbered spots are marked on the ground in it, and you pick the one {name} goes to, from what's been said."),
                format!("Here's what {player} is looking at. Which spot does {name} go to? Answer with \"place\": what's at that spot, in a few words, and \"spot\": its number (0 if none fits)."),
            )
        } else {
            (
                format!("{eyes} Say plainly what's there, in a sentence or two: what you see, not anyone's lines."),
                format!("Here's what {player} is looking at. What's there?"),
            )
        };
        let newest = self.memory.feed[self.memory.feed.len().saturating_sub(12)..].join("\n");
        let question = format!(
            "{}\n\nWhat's been happening, newest last:\n{newest}\n\n{question}",
            self.right_now()
        );
        let mut request = json!({
            "model": self.cfg.llm.model,
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": ""}],
            "temperature": self.cfg.llm.temperature,
            "stream": false,
        });
        if choose {
            let spots: Vec<u64> = std::iter::once(0)
                .chain(view.spots.iter().filter_map(|s| s["id"].as_u64()))
                .collect();
            request["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "spot", "schema": {
                "type": "object",
                "properties": {"place": {"type": "string", "maxLength": 80}, "spot": {"type": "integer", "enum": spots}},
                "required": ["place", "spot"],
            }}});
        }
        // (the picture in the request, and as traced: by its file, not its data)
        let mut traced = request.clone();
        for (request, url) in [
            (
                &mut request,
                format!("data:image/png;base64,{}", picture::base64(&png)),
            ),
            (&mut traced, format!("/view?name={file}")),
        ] {
            request["messages"][1]["content"] = json!([{"type": "text", "text": question}, {"type": "image_url", "image_url": {"url": url}}]);
        }
        let output = match self.agent.post(&self.cfg.llm.url).send_json(&request) {
            Ok(response) => response.into_json::<Value>()?["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string(),
            Err(e) => {
                let error = match e {
                    ureq::Error::Status(code, response) => {
                        format!("{code}: {}", response.into_string().unwrap_or_default())
                    }
                    other => other.to_string(),
                };
                eprintln!("[sight] {kind} failed: {error}");
                self.trace.add(
                    "sight",
                    json!({"kind": kind, "request": traced, "picture": file, "error": error}),
                );
                return Ok(("he can't see it".to_string(), json!(file)));
            }
        };
        let what = if choose {
            let answer: Value = serde_json::from_str(&output).unwrap_or_default();
            let spot = answer["spot"].as_u64().unwrap_or(0);
            match view
                .spots
                .iter()
                .find(|s| spot > 0 && s["id"].as_u64() == Some(spot))
            {
                Some(found) => {
                    msg["option"] = json!(spot.to_string());
                    self.link.send_json(msg);
                    format!(
                        "{} ({} m off)",
                        answer["place"].as_str().unwrap_or("there").trim(),
                        found["metres"]
                    )
                }
                None => "none that fits".to_string(),
            }
        } else {
            clean(&output)
        };
        self.trace.add(
            "sight",
            json!({
                "kind": kind,
                "url": self.cfg.llm.url,
                "request": traced,
                "output": output,
                "seen": what,
                "picture": file,
                "spots": view.spots.len(),
                "ms": start.elapsed().as_millis() as u64,
            }),
        );
        println!(
            "[sight] {kind}: {what} ({} ms)",
            start.elapsed().as_millis()
        );
        Ok((what, json!(file)))
    }

    /// Asks Kev the questions about `input`; the call, what went in and came out, goes in `calls`.
    fn kev(
        &self,
        purpose: &str,
        input: &str,
        questions: &Value,
        calls: &mut Vec<Value>,
    ) -> Result<Value> {
        let start = Instant::now();
        let request = json!({"model": self.cfg.kev.model, "state": input, "questions": questions});
        let result = self
            .kev_agent
            .post(&self.cfg.kev.url)
            .send_json(&request)
            .map_err(anyhow::Error::from)
            .and_then(|response| response.into_json().map_err(anyhow::Error::from));
        let resp: Value = match result {
            Ok(response) => response,
            Err(error) => {
                calls.push(
                    json!({"purpose": purpose, "url": self.cfg.kev.url, "request": request,
                    "error": format!("{error:#}"), "ms": start.elapsed().as_millis() as u64}),
                );
                return Err(error);
            }
        };
        calls.push(json!({
            "purpose": purpose,
            "url": self.cfg.kev.url,
            "request": request,
            "response": resp,
            "ms": start.elapsed().as_millis() as u64,
        }));
        Ok(resp["answers"].clone())
    }

    /// Streams an LLM line for the mate, speaking each sentence as soon as it's complete, and
    /// adds it to the feed.
    fn say(&mut self) -> Result<()> {
        let name = self.cfg.mate.name.clone();
        let messages = self.messages();
        let start = Instant::now();
        let generation = self.speaking.generation();
        let request = json!({"model": self.cfg.llm.model, "messages": messages, "temperature": self.cfg.llm.temperature, "stream": true});
        let resp = self
            .agent
            .post(&self.cfg.llm.url)
            .send_json(&request)
            .map_err(|e| {
                let detail = match e {
                    ureq::Error::Status(code, response) => format!(
                        "HTTP {code}: {}",
                        response.into_string().unwrap_or_default()
                    ),
                    other => other.to_string(),
                };
                let error = format!(
                    "LLM at {} (model {:?}): {detail}",
                    self.cfg.llm.url, self.cfg.llm.model
                );
                self.trace.add(
                    "llm",
                    json!({
                        "url": self.cfg.llm.url,
                        "model": self.cfg.llm.model,
                        "request": request,
                        "output": "",
                        "line": "",
                        "directions": [],
                        "first_token_ms": null,
                        "ms": start.elapsed().as_millis() as u64,
                        "cut_off": self.speaking.generation() != generation,
                        "error": error,
                    }),
                );
                anyhow::anyhow!(error)
            })?;
        // (he's talking from here until speak.rs has voiced the End below: always sent)
        self.speaking.begin_line();
        let mut failed = None;
        // (for the history page: the model the server named, and when its first words came)
        let mut model = Value::Null;
        let mut first_token_ms = Value::Null;

        let mut full = String::new();
        let mut notes = Notes::default();
        let mut splitter = SentenceSplitter::default();
        // How he says it: the latest "[...]" note, for the voice; each holds until the next.
        let mut how = String::new();
        let mut directions: Vec<String> = Vec::new();
        let speak = |sentence: String, how: &str| {
            let sentence = sentence.trim().to_string();
            if !sentence.is_empty() {
                let _ =
                    self.speech
                        .send(SpeechJob::Sentence(sentence, how.to_string(), generation));
            }
        };
        for line in BufReader::new(resp.into_reader()).lines() {
            if self.speaking.generation() != generation {
                // The player started talking; drop the rest.
                break;
            }
            let line = match line {
                Ok(line) => line,
                Err(e) => {
                    failed = Some(anyhow::Error::from(e));
                    break;
                }
            };
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            if data.trim() == "[DONE]" {
                break;
            }
            let chunk: Value = match serde_json::from_str(data) {
                Ok(chunk) => chunk,
                Err(e) => {
                    failed = Some(anyhow::Error::from(e));
                    break;
                }
            };
            if model.is_null() {
                model = chunk["model"].clone();
            }
            if let Some(delta) = chunk["choices"][0]["delta"]["content"].as_str() {
                if first_token_ms.is_null() {
                    first_token_ms = json!(start.elapsed().as_millis() as u64);
                }
                full.push_str(delta);
                for piece in notes.push(delta) {
                    match piece {
                        Piece::Text(text) => {
                            for sentence in splitter.push(&text) {
                                speak(sentence, &how);
                            }
                        }
                        Piece::Note(note) => {
                            // (what came before it is said as it was: a note mid-sentence splits it)
                            if let Some(rest) = splitter.finish() {
                                speak(rest, &how);
                            }
                            how = note;
                            directions.push(how.clone());
                        }
                    }
                }
            }
        }
        if let Some(rest) = splitter.finish() {
            speak(rest, &how);
        }
        let _ = self.speech.send(SpeechJob::End(generation));
        // (his turn as he wrote it, every note and paragraph: as the LLM sees its own lines)
        let line = full.trim().to_string();
        println!(
            "[llm] {name}: \"{line}\" ({} ms)",
            start.elapsed().as_millis()
        );
        self.trace.add(
            "llm",
            json!({
                "url": self.cfg.llm.url,
                "model": model,
                "request": request,
                "output": full,
                "directions": directions,
                "line": line,
                "first_token_ms": first_token_ms,
                "ms": start.elapsed().as_millis() as u64,
                "cut_off": self.speaking.generation() != generation,
                "error": failed.as_ref().map(|e| format!("{e:#}")),
            }),
        );
        self.memory.feed.push(format!("{name}: {line}"));
        failed.map_or(Ok(()), Err)
    }
}

/// Splits streamed text into speakable sentences.
#[derive(Default)]
struct SentenceSplitter {
    buf: String,
}

impl SentenceSplitter {
    fn push(&mut self, text: &str) -> Vec<String> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        loop {
            let chars: Vec<(usize, char)> = self.buf.char_indices().collect();
            let mut cut = None;
            for w in chars.windows(2) {
                let ((i, c), (_, next)) = (w[0], w[1]);
                let ends = matches!(c, '.' | '!' | '?' | '…') && next.is_whitespace() || c == '\n';
                if ends {
                    cut = Some(i + c.len_utf8());
                    break;
                }
            }
            let Some(cut) = cut else { break };
            let sentence = clean(&self.buf[..cut]);
            self.buf.drain(..cut);
            if !sentence.is_empty() {
                out.push(sentence);
            }
        }
        out
    }

    fn finish(&mut self) -> Option<String> {
        let rest = clean(&std::mem::take(&mut self.buf));
        (!rest.is_empty()).then_some(rest)
    }
}

/// The sounds the voice makes (Breeze TTS's four), by the ways the LLM writes them in a line:
/// "(sighs)" is said as "(sigh)". Anything else in round brackets isn't said.
const SOUNDS: [(&str, &str); 19] = [
    ("laugh", "(laugh)"),
    ("laughs", "(laugh)"),
    ("laughing", "(laugh)"),
    ("laughter", "(laugh)"),
    ("chuckle", "(laugh)"),
    ("chuckles", "(laugh)"),
    ("chuckling", "(laugh)"),
    ("giggles", "(laugh)"),
    ("snickers", "(laugh)"),
    ("cackles", "(laugh)"),
    ("sigh", "(sigh)"),
    ("sighs", "(sigh)"),
    ("sighing", "(sigh)"),
    ("cough", "(cough)"),
    ("coughs", "(cough)"),
    ("coughing", "(cough)"),
    ("clears throat", "(clears throat)"),
    ("clears his throat", "(clears throat)"),
    ("clearing throat", "(clears throat)"),
];

/// A piece of the LLM's streamed line: what he says, or a "[how he says it]" note, which holds
/// for what follows until the next.
enum Piece {
    Text(String),
    Note(String),
}

/// Splits the streamed line into its pieces, holding back a note until it's closed.
#[derive(Default)]
struct Notes {
    buf: String,
}

impl Notes {
    fn push(&mut self, text: &str) -> Vec<Piece> {
        self.buf.push_str(text);
        let mut pieces = Vec::new();
        loop {
            let Some(open) = self.buf.find('[') else {
                if !self.buf.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut self.buf)));
                }
                break;
            };
            if open > 0 {
                pieces.push(Piece::Text(self.buf[..open].to_string()));
                self.buf.drain(..open);
            }
            let Some(close) = self.buf.find(']') else {
                break;
            };
            pieces.push(Piece::Note(self.buf[1..close].trim().to_string()));
            self.buf.drain(..=close);
        }
        pieces
    }
}

/// Strips things TTS shouldn't read out: markdown, emoji, wrapping quotes.
fn clean(text: &str) -> String {
    // Drop stage directions like "[looks around]" or "(nods)", but not the sounds the voice
    // makes; an unclosed one runs to the end.
    let mut depth = 0u32;
    let mut unbracketed = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if depth == 0 && c == '(' {
            let sound = rest[1..].find(')').and_then(|end| {
                let written = rest[1..1 + end].trim().to_lowercase();
                SOUNDS
                    .iter()
                    .find(|(as_written, _)| *as_written == written)
                    .map(|(_, said)| (said, end + 2))
            });
            if let Some((said, length)) = sound {
                unbracketed.push_str(said);
                rest = &rest[length..];
                continue;
            }
        }
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => unbracketed.push(c),
            _ => {}
        }
        rest = &rest[c.len_utf8()..];
    }
    let filtered: String = unbracketed
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '#' | '~' | '`'))
        .filter(|&c| {
            !(('\u{1F000}'..='\u{1FAFF}').contains(&c) || ('\u{2600}'..='\u{27BF}').contains(&c))
        })
        .collect();
    let collapsed = filtered.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed
        .trim_matches(|c: char| c == '"' || c == '\u{201C}' || c == '\u{201D}')
        .trim();
    if trimmed.chars().any(char::is_alphanumeric) {
        trimmed.to_string()
    } else {
        String::new()
    }
}

/// Unknown, malformed and legacy action answers are noncommands.
fn command_intent(answer: &Value) -> &str {
    match answer["intent"]["choice"].as_str() {
        Some(intent @ ("follow" | "stay" | "go_aim" | "hold_fire" | "resume_fire")) => intent,
        _ => "none",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use std::io::Write;
    use std::net::TcpStream;
    use std::sync::mpsc;

    fn fixture(
        server: &MockHttp,
        dir: &TestDir,
    ) -> (Brain, Receiver<SpeechJob>, Receiver<Input>, TcpStream) {
        let mut cfg = config();
        cfg.base_dir = dir.0.clone();
        cfg.mate.persona = None;
        cfg.llm.url = format!("{}/v1/chat/completions", server.url);
        cfg.kev.url = format!("{}/v1/systemone", server.url);
        let (link, peer) = GameLink::test_pair();
        let (speech, jobs) = mpsc::channel();
        let (inputs, results) = mpsc::channel();
        let prompt = PromptFile::load(dir.0.join("system_prompt.txt")).unwrap();
        let mut brain = Brain::new(
            cfg,
            link,
            speech,
            Speaking::new(),
            Trace::new(&dir.0),
            prompt.clone(),
            prompt.clone(),
            prompt,
            inputs,
        );
        brain.memory.feed.push("Player: Cover me!".into());
        (brain, jobs, results, peer)
    }

    #[test]
    fn streamed_dialogue_routes_model_and_carries_notes_to_saved_profile_tts() {
        let server = MockHttp::new(|request, stream| {
            if request.path == "/v1/audio/speech" {
                reply(stream, 200, PCM_HEADERS, &[0, 0]);
                return;
            }
            let mut sse = String::new();
            for delta in [
                "[yell",
                "ing over gunfire]Cover me! I'm moving! ",
                "[quietly]Stay low. ",
                "[]Okay. (nods) (laughs)",
            ] {
                sse.push_str(&format!("data: {}\n\n", json!({"model": "gemma4-12b-mtp-vision", "choices": [{"delta": {"content": delta}}]})));
            }
            sse.push_str("data: [DONE]\n\n");
            reply(
                stream,
                200,
                &[("Content-Type", "text/event-stream")],
                sse.as_bytes(),
            );
        });
        let dir = TestDir::new();
        let (mut brain, jobs, _, _peer) = fixture(&server, &dir);
        brain.say().unwrap();
        let request = server.request();
        assert_eq!(request.path, "/v1/chat/completions");
        assert_eq!(request.json()["model"], "gemma4-12b-mtp-vision");
        assert_eq!(request.json()["stream"], true);
        let mut cfg = brain.cfg.clone();
        cfg.mate.voice_server = server.url.clone();
        let voice = crate::speak::Voice::load(&cfg).unwrap();
        let mut sentences = Vec::new();
        let mut ended = false;
        for job in jobs.try_iter() {
            match job {
                SpeechJob::Sentence(text, how, generation) => {
                    assert_eq!(generation, 0);
                    voice.stream(&text, &how, || true, |_| true).unwrap();
                    let body = server.request().json();
                    assert_eq!(
                        body["guidance_scale"],
                        if how.is_empty() { 1.0 } else { 4.0 }
                    );
                    if how.is_empty() {
                        assert!(body.get("instructions").is_none());
                    } else {
                        assert_eq!(body["instructions"], how);
                    }
                    sentences.push((text, how));
                }
                SpeechJob::End(0) => ended = true,
                _ => panic!("wrong generation"),
            }
        }
        assert!(ended);
        assert_eq!(
            sentences,
            [
                ("Cover me!".into(), "yelling over gunfire".into()),
                ("I'm moving!".into(), "yelling over gunfire".into()),
                ("Stay low.".into(), "quietly".into()),
                ("Okay.".into(), "".into()),
                ("(laugh)".into(), "".into()),
            ]
        );
        let entry: Value = serde_json::from_str(
            std::fs::read_to_string(
                std::fs::read_dir(&dir.0)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path(),
            )
            .unwrap()
            .lines()
            .last()
            .unwrap(),
        )
        .unwrap();
        assert!(entry["data"]["first_token_ms"].is_number());
    }

    #[test]
    fn rejected_dialogue_requests_are_recorded_with_router_error() {
        let server = MockHttp::new(|_, stream| {
            reply(
                stream,
                404,
                &[("Content-Type", "application/json")],
                br#"{"error":{"message":"no router for requested model"}}"#,
            )
        });
        let dir = TestDir::new();
        let (mut brain, jobs, _, _peer) = fixture(&server, &dir);
        let error = brain.say().unwrap_err();
        assert!(error.to_string().contains("HTTP 404"));
        assert!(error.to_string().contains("no router for requested model"));
        assert!(jobs.try_recv().is_err());
        assert!(brain.speaking.quiet_from().is_some());
        let log = std::fs::read_dir(&dir.0)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let entry: Value = serde_json::from_str(
            std::fs::read_to_string(log)
                .unwrap()
                .lines()
                .last()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(entry["kind"], "llm");
        assert!(entry["data"]["error"]
            .as_str()
            .unwrap()
            .contains("no router for requested model"));
        assert_eq!(entry["data"]["request"]["model"], "gemma4-12b-mtp-vision");
    }

    #[test]
    fn vision_description_and_schema_spot_selection_route_model() {
        let server = MockHttp::new(|request, stream| {
            let content = if request.json().get("response_format").is_some() {
                r#"{"place":"cover","spot":7}"#
            } else {
                "A wall ahead."
            };
            let body = json!({"choices": [{"message": {"content": content}}]}).to_string();
            reply(
                stream,
                200,
                &[("Content-Type", "application/json")],
                body.as_bytes(),
            );
        });
        for kind in ["describe", "choose"] {
            let dir = TestDir::new();
            let (mut brain, _, _, mut peer) = fixture(&server, &dir);
            let game = std::thread::spawn(move || {
                assert_eq!(
                    serde_json::from_slice::<Value>(&frame(&mut peer).1).unwrap()["type"],
                    "view"
                );
                let about = json!({"width": 2, "height": 2, "spots": [{"id": 7, "x": 0.5, "y": 0.5, "metres": 3}]}).to_string();
                let mut payload = (about.len() as u16).to_be_bytes().to_vec();
                payload.extend(about.as_bytes());
                payload.extend([80; 12]);
                peer.write_all(&[4]).unwrap();
                peer.write_all(&(payload.len() as u32).to_be_bytes())
                    .unwrap();
                peer.write_all(&payload).unwrap();
                if kind == "choose" {
                    let (frame_kind, body) = frame(&mut peer);
                    assert_eq!(frame_kind, 1);
                    assert_eq!(
                        serde_json::from_slice::<Value>(&body).unwrap()["option"],
                        "7"
                    );
                }
            });
            let mut action = json!({"type": "action", "action": "go_to"});
            let (seen, _) = brain.look(kind, &mut action).unwrap();
            assert!(seen.contains(if kind == "choose" { "cover" } else { "wall" }));
            game.join().unwrap();
            let body = server.request().json();
            assert_eq!(body["model"], "gemma4-12b-mtp-vision");
            assert_eq!(body["stream"], false);
            assert!(body["messages"][1]["content"][1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,"));
            if kind == "choose" {
                assert_eq!(body["response_format"]["type"], "json_schema");
                assert_eq!(
                    body["response_format"]["json_schema"]["schema"]["properties"]["spot"]["enum"],
                    json!([0, 7])
                );
            } else {
                assert!(body.get("response_format").is_none());
            }
        }
    }

    #[test]
    fn memory_summary_routes_configured_model_on_background_thread() {
        let server = MockHttp::new(|_, stream| {
            reply(
                stream,
                200,
                &[("Content-Type", "application/json")],
                br#"{"choices":[{"message":{"content":"Kevin reached cover."}}]}"#,
            );
        });
        let dir = TestDir::new();
        let (mut brain, _, results, _peer) = fixture(&server, &dir);
        brain.cfg.llm.model = "custom-gemma-route".into();
        brain.memory.feed = vec!["A long story. ".repeat(FEED_TOKENS)];
        brain.fold_over(FEED_TOKENS);
        match results.recv_timeout(Duration::from_secs(3)).unwrap() {
            Input::Summary {
                text,
                folded,
                memory,
            } => {
                assert_eq!(text, "Kevin reached cover.");
                assert_eq!(folded, 1);
                assert_eq!(memory, 0);
            }
            _ => panic!("expected summary"),
        }
        let body = server.request().json();
        assert_eq!(body["model"], "custom-gemma-route");
        assert_eq!(body["temperature"], 0.3);
        assert!(body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("at most 250 words"));
    }

    #[test]
    fn system_one_preserves_questions_and_answers_with_configured_model() {
        let answers = json!({"speak": {"noul": 0.8}, "do": {"choice": "follow"}});
        let response = json!({"answers": answers}).to_string();
        let server = MockHttp::new(move |_, stream| {
            reply(
                stream,
                200,
                &[("Content-Type", "application/json")],
                response.as_bytes(),
            )
        });
        let dir = TestDir::new();
        let (mut brain, _, _, _peer) = fixture(&server, &dir);
        let questions = json!({
            "speak": {"type": "noul", "instructions": "Should Kevin speak now?"},
            "do": {"type": "choice", "instructions": "What does Kevin do?", "criteria": {"follow": "Kevin follows", "shoot": "Kevin shoots"}}
        });
        for model in ["clef-flash-artemis", "custom-clef-route"] {
            brain.cfg.kev.model = model.into();
            let result = brain
                .kev("test", "Player needs cover", &questions, &mut Vec::new())
                .unwrap();
            assert_eq!(result, answers);
            let request = server.request();
            assert_eq!(request.path, "/v1/systemone");
            assert_eq!(
                request.json(),
                json!({"model": model, "state": "Player needs cover", "questions": questions})
            );
        }
    }

    #[test]
    fn events_only_ask_about_speech_and_cooldown_skips_all_clef_calls() {
        let server = MockHttp::new(|request, stream| {
            let body = request.json();
            assert_eq!(body["questions"].as_object().unwrap().len(), 1);
            assert_eq!(body["questions"]["speak"]["type"], "noul");
            reply(
                stream,
                200,
                &[("Content-Type", "application/json")],
                br#"{"answers":{"speak":{"noul":0.8}}}"#,
            );
        });
        let dir = TestDir::new();
        let (mut brain, _, _, _peer) = fixture(&server, &dir);
        *brain.link.state.lock().unwrap() = json!({"actions": [{"name": "shoot"}]});
        assert!(brain.decide(false, false).unwrap());
        server.request();
        assert!(!brain.decide(false, true).unwrap());
        assert!(brain.decide(true, false).unwrap());
        assert!(server.requests.try_recv().is_err());
    }

    #[test]
    fn malformed_or_legacy_intents_are_none() {
        for answer in [
            Value::Null,
            json!({"intent":{"choice":"shoot"}}),
            json!({"intent":{"choice":12}}),
            json!({"do":{"choice":"follow"}}),
        ] {
            assert_eq!(command_intent(&answer), "none");
        }
        for intent in [
            "none",
            "follow",
            "stay",
            "go_aim",
            "hold_fire",
            "resume_fire",
        ] {
            assert_eq!(command_intent(&json!({"intent":{"choice":intent}})), intent);
        }
    }

    #[test]
    fn ordinary_conversation_makes_one_focused_choice_and_sends_no_order() {
        let server = MockHttp::new(|request, stream| {
            let body = request.json();
            assert_eq!(body["questions"].as_object().unwrap().len(), 1);
            let criteria = body["questions"]["intent"]["criteria"].as_object().unwrap();
            assert_eq!(criteria.len(), 6);
            assert!(criteria.contains_key("none"));
            assert!(!body["state"].as_str().unwrap().contains("Cover me!"));
            reply(
                stream,
                200,
                &[("Content-Type", "application/json")],
                br#"{"answers":{"intent":{"choice":"none"}}}"#,
            );
        });
        let dir = TestDir::new();
        let (mut brain, _, _, mut peer) = fixture(&server, &dir);
        brain
            .interpret_command("Do you remember home?", Some(123))
            .unwrap();
        server.request();
        assert!(server.requests.try_recv().is_err());
        peer.set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        use std::io::Read;
        assert!(peer.read(&mut [0; 1]).is_err());
    }

    #[test]
    fn command_uses_captured_context_and_waits_for_native_result() {
        let server = MockHttp::new(|_, stream| {
            reply(
                stream,
                200,
                &[("Content-Type", "application/json")],
                br#"{"answers":{"intent":{"choice":"go_aim"}}}"#,
            )
        });
        let dir = TestDir::new();
        let (mut brain, _, _, mut peer) = fixture(&server, &dir);
        let native = std::thread::spawn(move || {
            let request: Value = serde_json::from_slice(&frame(&mut peer).1).unwrap();
            assert_eq!(request["action"], "go_aim");
            assert_eq!(request["context_id"], 71);
            let result = json!({"type":"command_result", "request_id":request["request_id"],
                "accepted":false,"text":"Unsafe landing."})
            .to_string();
            peer.write_all(&[1]).unwrap();
            peer.write_all(&(result.len() as u32).to_be_bytes())
                .unwrap();
            peer.write_all(result.as_bytes()).unwrap();
        });
        brain.interpret_command("Go there.", Some(71)).unwrap();
        native.join().unwrap();
        assert!(brain.memory.feed.last().unwrap().contains("Unsafe landing"));
        assert!(brain
            .memory
            .feed
            .last()
            .unwrap()
            .contains("previous orders retained"));
    }

    fn trace_entries(dir: &TestDir) -> Vec<Value> {
        let log = std::fs::read_dir(&dir.0)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
            .unwrap();
        std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn natural_language_intents_follow_clef_without_a_literal_guard() {
        for (text, intent) in [
            ("Kevin, get over here", "follow"),
            ("Can you wait for me?", "stay"),
            ("Head over where I'm pointing", "go_aim"),
            ("Keep your finger off the trigger", "hold_fire"),
            ("You're clear to engage again", "resume_fire"),
        ] {
            let server = MockHttp::new(move |request, stream| {
                let body = request.json();
                assert_eq!(
                    body["state"],
                    format!(
                        "Current player utterance: {}",
                        serde_json::to_string(text).unwrap()
                    )
                );
                assert_eq!(body["questions"].as_object().unwrap().len(), 1);
                let instructions = body["questions"]["intent"]["instructions"]
                    .as_str()
                    .unwrap();
                for word in [
                    "conversation",
                    "hypothetical",
                    "quoted",
                    "ambiguous",
                    "conflicting",
                    "unsupported",
                ] {
                    assert!(instructions.contains(word), "{word}");
                }
                reply(stream, 200, &[], json!({"answers":{"intent":{"choice":intent,"confidence":0.7,"probabilities":{"none":0.3}}}}).to_string().as_bytes());
            });
            let dir = TestDir::new();
            let (mut brain, _, _, mut peer) = fixture(&server, &dir);
            let native = std::thread::spawn(move || {
                let action: Value = serde_json::from_slice(&frame(&mut peer).1).unwrap();
                assert_eq!(action["action"], intent);
                assert_eq!(action["context_id"], 71);
                let response = json!({"type":"command_result","request_id":action["request_id"],"accepted":true,
                    "movement_order":"follow","fire_enabled":true}).to_string();
                peer.write_all(&[1]).unwrap();
                peer.write_all(&(response.len() as u32).to_be_bytes())
                    .unwrap();
                peer.write_all(response.as_bytes()).unwrap();
            });
            brain.interpret_command(text, Some(71)).unwrap();
            native.join().unwrap();
            assert!(brain.memory.feed.last().unwrap().contains("Kevin is now"));
            let entries = trace_entries(&dir);
            assert_eq!(
                entries.iter().find(|e| e["kind"] == "kev").unwrap()["data"]["calls"][0]
                    ["response"]["answers"]["intent"]["confidence"],
                0.7
            );
        }
    }

    // Exercise the actual sequential run loop with an utterance arriving while an
    // HTTP request is blocked. The native mock also captures aim at enqueue time.
    fn queued_utterances(delay_at: &str, timeout: bool, stale: bool) {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let delayed_dialogue = delay_at == "dialogue";
        let delay_at = delay_at.to_string();
        let mut delayed = false;
        let mut intents = 0;
        let server = MockHttp::new(move |request, stream| {
            let body = request.json();
            let intent = !body["questions"]["intent"].is_null();
            if !delayed
                && ((delay_at == "intent" && intent)
                    || (delay_at == "dialogue" && request.path.ends_with("completions")))
            {
                delayed = true;
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                if timeout {
                    intents += 1;
                    return;
                }
            }
            if intent {
                intents += 1;
                reply(
                    stream,
                    200,
                    &[],
                    json!({"answers":{"intent":{"choice":if intents==1 {"go_aim"} else {"stay"}}}})
                        .to_string()
                        .as_bytes(),
                );
            } else if !body["questions"]["speak"].is_null() {
                reply(stream, 200, &[], br#"{"answers":{"speak":{"noul":0}}}"#);
            } else {
                reply(stream,200,&[("Content-Type","text/event-stream")], b"data: {\"choices\":[{\"delta\":{\"content\":\"Okay.\"}}]}\n\ndata: [DONE]\n\n");
            }
        });
        let dir = TestDir::new();
        let (mut brain, _jobs, inputs_rx, mut peer) = fixture(&server, &dir);
        brain.kev_agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_millis(150))
            .timeout_connect(Duration::from_millis(100))
            .build();
        let inputs = brain.inputs.clone();
        let link = brain.link.clone();
        let (actions_tx, actions_rx) = mpsc::channel();
        let native = std::thread::spawn(move || {
            let mut captured = 0;
            let mut actions = 0;
            loop {
                let request: Value = serde_json::from_slice(&frame(&mut peer).1).unwrap();
                let response=match request["type"].as_str() {
                    Some("capture_command")=> { captured+=1;json!({"type":"command_context","request_id":request["request_id"],"context_id":70+captured}) },
                    Some("action")=> {
                        assert!(request.get("context_id").is_some());actions+=1;actions_tx.send(request.clone()).unwrap();
                        json!({"type":"command_result","request_id":request["request_id"],"accepted":!stale,
                            "text":"Captured context expired; previous orders retained.","movement_order":"stay","fire_enabled":true})
                    },
                    _=>continue,
                }.to_string();
                peer.write_all(&[1]).unwrap();
                peer.write_all(&(response.len() as u32).to_be_bytes())
                    .unwrap();
                peer.write_all(response.as_bytes()).unwrap();
                if actions == if timeout { 1 } else { 2 } {
                    assert_eq!(
                        captured, 2,
                        "never recapture when processing queued commands"
                    );
                    break;
                }
            }
        });
        let event_budget = EventBudget::default();
        for _ in 0..100 {
            event_budget.send(&inputs, "Repeated game event".into(), false);
        }
        inputs
            .send(Input::heard("Head over where I'm pointing".into(), &link))
            .unwrap();
        let runner = std::thread::spawn(move || brain.run(inputs_rx));
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        inputs
            .send(Input::heard("Can you wait for me?".into(), &link))
            .unwrap();
        // A later event can only ask whether to speak; it cannot change the order.
        let budget = EventBudget::default();
        budget.send(&inputs, "An enemy moved nearby".into(), true);
        inputs.send(Input::Stop).unwrap();
        if timeout || delayed_dialogue {
            std::thread::sleep(Duration::from_millis(220));
        }
        release_tx.send(()).unwrap();
        runner.join().unwrap();
        native.join().unwrap();
        let actions: Vec<_> = actions_rx.try_iter().collect();
        assert_eq!(actions.len(), if timeout { 1 } else { 2 });
        if !timeout {
            assert_eq!(actions[0]["context_id"], 71);
            assert_eq!(actions[0]["action"], "go_aim");
        }
        assert_eq!(actions.last().unwrap()["context_id"], 72);
        assert_eq!(actions.last().unwrap()["action"], "stay");
        let entries = trace_entries(&dir);
        let heard: Vec<_> = entries
            .iter()
            .filter(|e| e["kind"] == "heard")
            .map(|e| e["data"]["text"].as_str().unwrap())
            .collect();
        assert_eq!(
            heard,
            vec!["Head over where I'm pointing", "Can you wait for me?"]
        );
        assert_eq!(entries.iter().filter(|e| e["kind"] == "llm").count(), 2);
        if timeout {
            let failed = entries
                .iter()
                .find(|e| e["kind"] == "kev" && !e["data"]["error"].is_null())
                .unwrap();
            assert!(failed["data"]["ms"].as_u64().unwrap() < 600);
            assert!(failed["data"]["calls"][0]["error"].is_string());
        }
        if stale {
            assert!(entries
                .iter()
                .filter(|e| e["kind"] == "command")
                .all(|e| e["data"]["text"]
                    .as_str()
                    .unwrap()
                    .contains("previous orders retained")));
        }
    }

    #[test]
    fn utterance_queued_during_intent_keeps_its_own_context() {
        queued_utterances("intent", false, false);
    }
    #[test]
    fn utterance_queued_during_dialogue_keeps_its_own_context() {
        queued_utterances("dialogue", false, false);
    }
    #[test]
    fn intent_timeout_preserves_later_input_and_dialogue() {
        queued_utterances("intent", true, false);
    }
    #[test]
    fn stale_backlogged_commands_are_rejected_without_recapture() {
        queued_utterances("dialogue", false, true);
    }

    #[test]
    fn event_budget_bounds_batches_without_losing_player_input() {
        let budget = EventBudget::default();
        let (tx, rx) = mpsc::channel();
        for _ in 0..10000 {
            budget.send(&tx, "repeated event".into(), true);
        }
        tx.send(Input::Heard {
            text: "Stay here".into(),
            context: Some(73),
        })
        .unwrap();
        let queued: Vec<_> = rx.try_iter().collect();
        assert_eq!(queued.len(), 65);
        assert_eq!(budget.0.pending.load(Ordering::Relaxed), 64);
        assert_eq!(budget.0.dropped.load(Ordering::Relaxed), 9936);
        assert!(matches!(
            queued.last(),
            Some(Input::Heard {
                context: Some(73),
                ..
            })
        ));
        drop(queued);
        assert_eq!(budget.0.pending.load(Ordering::Relaxed), 0);
        budget.send(&tx, "new event".into(), true);
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn event_timeout_skips_optional_speech_and_traces_failure() {
        let server = MockHttp::new(|_, _| std::thread::sleep(Duration::from_millis(120)));
        let dir = TestDir::new();
        let (mut brain, _, _, _peer) = fixture(&server, &dir);
        brain.kev_agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_millis(40))
            .build();
        let start = Instant::now();
        assert!(!brain.decide(false, false).unwrap());
        assert!(start.elapsed() < Duration::from_millis(500));
        let entries = trace_entries(&dir);
        assert!(entries[0]["data"]["calls"][0]["error"].is_string());
        assert_eq!(entries[0]["data"]["speak"], false);
    }

    #[test]
    fn supported_sounds_and_stage_directions_remain_unchanged() {
        assert_eq!(
            clean("(nods) Ready. (sighs) (laughing) (coughs) (clears his throat)"),
            "Ready. (sigh) (laugh) (cough) (clears throat)"
        );
    }
}
