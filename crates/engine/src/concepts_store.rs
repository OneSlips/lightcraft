//! Remembered objects ("Wedding dress", "Groom's suit"): what the photographer taught, kept in the
//! library as `concepts.json` (local only). Each example is opaque JSON here
//! (`lightcraft_segment::concept::Example`: SAM 2.1 feature prototypes and colours of one taught
//! selection; the AI crate isn't in every build).

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const FILE: &str = "concepts.json";
const MAX_BYTES: usize = 64 << 20;
/// Examples a concept keeps (the oldest go first).
pub const MAX_EXAMPLES: usize = 12;
const MAX_CONCEPTS: usize = 500;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Concept {
    pub name: String,
    #[serde(default)]
    pub examples: Vec<Value>,
    /// The photos it was taught on (for the list; ids).
    #[serde(default)]
    pub photos: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConceptStore {
    pub version: u32,
    #[serde(default)]
    pub concepts: Vec<Concept>,
}

impl Default for ConceptStore {
    fn default() -> Self {
        ConceptStore { version: 1, concepts: Vec::new() }
    }
}

fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

impl ConceptStore {
    pub fn from_bytes(bytes: &[u8]) -> Result<ConceptStore, String> {
        if bytes.len() > MAX_BYTES {
            return Err(format!("{FILE} is unexpectedly large ({} MB)", bytes.len() >> 20));
        }
        let mut s: ConceptStore = serde_json::from_slice(bytes).map_err(|e| format!("{FILE} is damaged: {e}"))?;
        s.concepts.truncate(MAX_CONCEPTS);
        for c in &mut s.concepts {
            let n = c.examples.len();
            if n > MAX_EXAMPLES {
                c.examples.drain(..n - MAX_EXAMPLES);
            }
        }
        Ok(s)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn get(&self, name: &str) -> Option<&Concept> {
        self.concepts.iter().find(|c| same(&c.name, name))
    }

    /// Add an example to `name` (created when new). → how many examples it has.
    pub fn teach(&mut self, name: &str, photo: u64, example: Value) -> Result<usize, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            return Err("give the object a name (at most 120 characters)".into());
        }
        let i = match self.concepts.iter().position(|c| same(&c.name, name)) {
            Some(i) => i,
            None => {
                if self.concepts.len() >= MAX_CONCEPTS {
                    return Err("too many remembered objects".into());
                }
                self.concepts.push(Concept { name: name.into(), ..Default::default() });
                self.concepts.len() - 1
            }
        };
        let c = self.concepts.get_mut(i).ok_or("no such object")?;
        c.examples.push(example);
        if !c.photos.contains(&photo) {
            c.photos.push(photo);
        }
        let n = c.examples.len();
        if n > MAX_EXAMPLES {
            c.examples.drain(..n - MAX_EXAMPLES);
        }
        Ok(c.examples.len())
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        let to = to.trim();
        if to.is_empty() {
            return Err("missing name".into());
        }
        if !same(from, to) && self.get(to).is_some() {
            return Err(format!("an object called “{to}” exists already"));
        }
        let c = self.concepts.iter_mut().find(|c| same(&c.name, from)).ok_or_else(|| format!("no object called “{from}”"))?;
        c.name = to.into();
        Ok(())
    }

    pub fn forget(&mut self, name: &str) -> bool {
        let n = self.concepts.len();
        self.concepts.retain(|c| !same(&c.name, name));
        n != self.concepts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teach_rename_forget() {
        let mut s = ConceptStore::default();
        assert_eq!(s.teach("Wedding dress", 1, Value::Null).unwrap(), 1);
        assert_eq!(s.teach("wedding DRESS", 2, Value::Null).unwrap(), 2);
        for i in 0..20 {
            s.teach("Wedding dress", 3 + i, Value::Null).unwrap();
        }
        assert_eq!(s.get("Wedding dress").unwrap().examples.len(), MAX_EXAMPLES);
        assert!(s.teach(" ", 1, Value::Null).is_err());
        s.teach("Suit", 1, Value::Null).unwrap();
        assert!(s.rename("Suit", "wedding dress").is_err());
        s.rename("Suit", "Groom's suit").unwrap();
        assert!(s.get("groom's suit").is_some());
        assert!(s.forget("GROOM'S SUIT"));
        assert!(ConceptStore::from_bytes(b"[1,2").is_err());
        assert_eq!(ConceptStore::from_bytes(&s.to_bytes()).unwrap(), s);
    }
}
