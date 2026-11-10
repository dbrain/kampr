use super::{NO_WORDS, Pressed, decision, not_an_answer, option, worded};
use crate::pending::Pending;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

const TYPED: &str = "Type something";

#[derive(Debug)]
pub struct Ask {
    questions: Vec<Value>,
    at: usize,
    ticked: BTreeSet<usize>,
    answers: Map<String, Value>,
}

impl Ask {
    pub fn of(input: &Value) -> Option<Self> {
        let questions = input["questions"].as_array().filter(|q| !q.is_empty())?;
        Some(Self {
            questions: questions.clone(),
            at: 0,
            ticked: BTreeSet::new(),
            answers: Map::new(),
        })
    }

    pub fn step(&self) -> Option<usize> {
        (self.questions.len() > 1).then_some(self.at)
    }

    fn asked(&self) -> &Value {
        &self.questions[self.at]
    }

    fn multi(&self) -> bool {
        self.asked()["multiSelect"].as_bool().unwrap_or(false)
    }

    pub fn view(&self) -> Pending {
        let asked = self.asked();
        let multi = self.multi();
        let header = asked["header"].as_str().map(str::to_string);
        let header = match self.questions.len() {
            1 => header,
            n => Some(format!(
                "{} · {} of {n}",
                header.unwrap_or_else(|| "Question".into()),
                self.at + 1
            )),
        };
        let mut options: Vec<_> = labels(asked)
            .enumerate()
            .map(|(i, (label, detail))| {
                option(
                    &(i + 1).to_string(),
                    label,
                    detail,
                    multi && self.ticked.contains(&i),
                )
            })
            .collect();
        options.push(worded(&(options.len() + 1).to_string(), TYPED));
        Pending {
            question: asked["question"].as_str().unwrap_or_default().to_string(),
            options,
            header,
            multi,
            cursor: None,
            context: None,
            id: None,
        }
    }

    pub fn press(&mut self, key: &str, text: Option<&str>) -> Result<Pressed, String> {
        let count = labels(self.asked()).count();
        let index = key
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .filter(|i| *i <= count)
            .ok_or_else(|| not_an_answer(key))?;
        if index == count {
            let typed = text.ok_or("that answer needs words")?;
            return Ok(self.commit(Some(typed)));
        }
        if text.is_some() {
            return Err(NO_WORDS.into());
        }
        if self.multi() {
            if !self.ticked.remove(&index) {
                self.ticked.insert(index);
            }
            return Ok(Pressed::Moved);
        }
        self.ticked.clear();
        self.ticked.insert(index);
        Ok(self.commit(None))
    }

    pub fn submit(&mut self) -> Result<Pressed, String> {
        if !self.multi() {
            return Err("this question takes one answer, and a press gives it".into());
        }
        match self.ticked.is_empty() {
            true => Err("nothing is ticked yet".into()),
            false => Ok(self.commit(None)),
        }
    }

    // Words the operator typed reach the model as they are, as the dialog's own "Type
    // something" would (#639); on a multiple-answer question they follow the ticked labels.
    fn commit(&mut self, typed: Option<&str>) -> Pressed {
        let asked = self.asked();
        let mut chosen: Vec<&str> = labels(asked)
            .enumerate()
            .filter(|(i, _)| self.ticked.contains(i))
            .map(|(_, (label, _))| label)
            .collect();
        chosen.extend(typed);
        // One string for several answers, as the keyboard records it (#634).
        let answer = chosen.join(", ");
        let question = asked["question"].as_str().unwrap_or_default().to_string();
        self.answers.insert(question, json!(answer));
        self.ticked.clear();
        self.at += 1;
        if self.at < self.questions.len() {
            return Pressed::Moved;
        }
        Pressed::Decided(decision(json!({
            "behavior": "allow",
            "updatedInput": { "questions": self.questions, "answers": self.answers },
        })))
    }
}

fn labels(question: &Value) -> impl Iterator<Item = (&str, Option<&str>)> {
    question["options"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|o| (o["label"].as_str().unwrap_or_default(), o["description"].as_str()))
}
