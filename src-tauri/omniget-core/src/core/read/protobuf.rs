use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Field<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed(u64),
}

pub fn varint(b: &[u8], p: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*p)?;
        *p += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

pub fn fields(b: &[u8]) -> HashMap<u32, Vec<Field<'_>>> {
    let mut out: HashMap<u32, Vec<Field<'_>>> = HashMap::new();
    let mut p = 0;
    while p < b.len() {
        let Some(key) = varint(b, &mut p) else { break };
        let tag = (key >> 3) as u32;
        let f = match key & 7 {
            0 => match varint(b, &mut p) {
                Some(v) => Field::Varint(v),
                None => break,
            },
            1 => {
                let Some(s) = b.get(p..p + 8) else { break };
                p += 8;
                Field::Fixed(u64::from_le_bytes(s.try_into().unwrap_or([0; 8])))
            }
            2 => {
                let Some(len) = varint(b, &mut p) else { break };
                let Some(s) = b.get(p..p + len as usize) else {
                    break;
                };
                p += len as usize;
                Field::Bytes(s)
            }
            5 => {
                let Some(s) = b.get(p..p + 4) else { break };
                p += 4;
                Field::Fixed(u32::from_le_bytes(s.try_into().unwrap_or([0; 4])) as u64)
            }
            _ => break,
        };
        out.entry(tag).or_default().push(f);
    }
    out
}

pub trait Msg<'a> {
    fn msg(&self, tag: u32) -> Option<HashMap<u32, Vec<Field<'a>>>>;
    fn msgs(&self, tag: u32) -> Vec<HashMap<u32, Vec<Field<'a>>>>;
    fn str(&self, tag: u32) -> String;
    fn num(&self, tag: u32) -> u64;
    fn has(&self, tag: u32) -> bool;
}

impl<'a> Msg<'a> for HashMap<u32, Vec<Field<'a>>> {
    fn msg(&self, tag: u32) -> Option<HashMap<u32, Vec<Field<'a>>>> {
        self.get(&tag)?.iter().find_map(|f| match f {
            Field::Bytes(b) => Some(fields(b)),
            _ => None,
        })
    }
    fn msgs(&self, tag: u32) -> Vec<HashMap<u32, Vec<Field<'a>>>> {
        self.get(&tag)
            .map(|v| {
                v.iter()
                    .filter_map(|f| match f {
                        Field::Bytes(b) => Some(fields(b)),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    fn str(&self, tag: u32) -> String {
        self.get(&tag)
            .and_then(|v| v.first())
            .and_then(|f| match f {
                Field::Bytes(b) => Some(String::from_utf8_lossy(b).to_string()),
                _ => None,
            })
            .unwrap_or_default()
    }
    fn num(&self, tag: u32) -> u64 {
        self.get(&tag)
            .and_then(|v| v.first())
            .map(|f| match f {
                Field::Varint(v) | Field::Fixed(v) => *v,
                _ => 0,
            })
            .unwrap_or(0)
    }
    fn has(&self, tag: u32) -> bool {
        self.contains_key(&tag)
    }
}

impl<'a> Field<'a> {
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Field::Fixed(v) => Some(f32::from_bits(*v as u32)),
            _ => None,
        }
    }
}

pub fn strings(m: &HashMap<u32, Vec<Field<'_>>>, tag: u32) -> Vec<String> {
    m.get(&tag)
        .map(|v| {
            v.iter()
                .filter_map(|f| match f {
                    Field::Bytes(b) => Some(String::from_utf8_lossy(b).to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn numbers(m: &HashMap<u32, Vec<Field<'_>>>, tag: u32) -> Vec<u64> {
    let mut out = Vec::new();
    for f in m.get(&tag).into_iter().flatten() {
        match f {
            Field::Varint(v) | Field::Fixed(v) => out.push(*v),
            Field::Bytes(b) => {
                let mut p = 0;
                while p < b.len() {
                    match varint(b, &mut p) {
                        Some(v) => out.push(v),
                        None => break,
                    }
                }
            }
        }
    }
    out
}
