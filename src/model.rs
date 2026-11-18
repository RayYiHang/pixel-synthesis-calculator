//! 游戏数据模型：加载并组织合成公式数据。
//!
//! 数据来自原 Windows 版合成计算器的加密 `.lml` 数据文件，已解密为 JSON 随应用内嵌。
//! 合成规则：3★+3★→4★、4★+4★→5★、5★+5★→7★。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// 服务器（数据源）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Server {
    /// 国际服（当前维护的数据源）。
    Global,
    /// 国服（已停止维护，数据仅供参考）。
    China,
}

impl Server {
    pub fn label(self) -> &'static str {
        match self {
            Server::Global => "国际服",
            Server::China => "国服",
        }
    }
}

/// 单个服务器的合成数据。
#[derive(Default)]
pub struct ServerData {
    /// CID → 星级（1-7）。
    pub cid_to_star: BTreeMap<u32, u8>,
    /// CID → 中文名（当前服务器）。
    pub cid_to_name: BTreeMap<u32, String>,
    /// CID → 英文名。
    pub cid_to_name_en: BTreeMap<u32, String>,
    /// 3★+3★ → 4★，键为规范化 (a, b)（a ≤ b）。主要供测试与扩展查询使用。
    #[allow(dead_code)]
    pub synth4: SynthMap,
    /// 4★+4★ → 5★。
    #[allow(dead_code)]
    pub synth5: SynthMap,
    /// 5★+5★ → 7★。
    #[allow(dead_code)]
    pub synth7: SynthMap,
    /// 逆向索引：4★ 结果 → 3★ 材料对列表。
    pub rev4: BTreeMap<u32, Vec<(u32, u32)>>,
    /// 逆向索引：5★ 结果 → 4★ 材料对列表。
    pub rev5: BTreeMap<u32, Vec<(u32, u32)>>,
    /// 逆向索引：7★ 结果 → 5★ 材料对列表。
    pub rev7: BTreeMap<u32, Vec<(u32, u32)>>,
    /// 5★ 目标 → 合法中间材料集合（3★ / 4★）。
    pub hccl5: HashMap<u32, HashSet<u32>>,
    /// 7★ 目标 → 合法中间材料集合（3★ / 4★ / 5★）。
    pub hccl7: HashMap<u32, HashSet<u32>>,
}

impl ServerData {
    /// 英雄名称（中文名优先，缺失时回退英文名）。
    pub fn name(&self, cid: u32) -> &str {
        self.cid_to_name
            .get(&cid)
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                self.cid_to_name_en
                    .get(&cid)
                    .map(|s| s.as_str())
                    .unwrap_or("?")
            })
    }

    pub fn star(&self, cid: u32) -> u8 {
        self.cid_to_star.get(&cid).copied().unwrap_or(0)
    }

    /// 某结果可由哪些材料对合成（按星级查对应逆向索引）。
    pub fn recipes_for(&self, star: u8) -> Option<&BTreeMap<u32, Vec<(u32, u32)>>> {
        match star {
            4 => Some(&self.rev4),
            5 => Some(&self.rev5),
            7 => Some(&self.rev7),
            _ => None,
        }
    }

    /// 目标的合法中间材料集合（4★ 目标无限制）。
    pub fn hccl_for(&self, target: u32, target_star: u8) -> Option<&HashSet<u32>> {
        match target_star {
            5 => self.hccl5.get(&target),
            7 => self.hccl7.get(&target),
            _ => None,
        }
    }
}

/// 全部数据（两个服务器），启动时加载一次。
pub struct GameData {
    pub global: Arc<ServerData>,
    pub china: Arc<ServerData>,
}

impl GameData {
    pub fn load() -> Self {
        GameData {
            global: Arc::new(load_server(
                include_str!("../assets/hcdata/cid_to_name_gjf.json"),
                include_str!("../assets/hcdata/x4_t_gjf.json"),
                include_str!("../assets/hcdata/x5_t_gjf.json"),
                include_str!("../assets/hcdata/x7_t_gjf.json"),
                include_str!("../assets/hcdata/hccl5_gjf.json"),
                include_str!("../assets/hcdata/hccl7_gjf.json"),
            )),
            china: Arc::new(load_server(
                include_str!("../assets/hcdata/cid_to_name_gf.json"),
                include_str!("../assets/hcdata/x4_t_gf.json"),
                include_str!("../assets/hcdata/x5_t_gf.json"),
                include_str!("../assets/hcdata/x7_t_gf.json"),
                include_str!("../assets/hcdata/hccl5_gf.json"),
                include_str!("../assets/hcdata/hccl7_gf.json"),
            )),
        }
    }

    pub fn server(&self, server: Server) -> Arc<ServerData> {
        match server {
            Server::Global => self.global.clone(),
            Server::China => self.china.clone(),
        }
    }
}

/// 正向合成表：`(a, b) → 结果`；逆向索引：`结果 → 材料对列表`。
pub type SynthMap = HashMap<(u32, u32), u32>;
pub type RecipeIndex = BTreeMap<u32, Vec<(u32, u32)>>;

/// 由 `_T` 逆向表构建 `键(a+b) → 结果` 与 `结果 → 材料对` 双向索引。
fn build_synth(t_json: &str) -> (SynthMap, RecipeIndex) {
    let raw: HashMap<String, String> = serde_json::from_str(t_json).expect("valid JSON");
    let mut synth = HashMap::with_capacity(raw.len());
    let mut rev: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
    for (key, value) in raw {
        let result: u32 = value.parse().expect("cid must be numeric");
        let mut it = key.splitn(2, '+');
        let a: u32 = it.next().unwrap_or_default().parse().expect("valid pair");
        let b: u32 = it.next().unwrap_or_default().parse().expect("valid pair");
        let canon = (a.min(b), a.max(b));
        synth.insert(canon, result);
        rev.entry(result).or_default().push(canon);
    }
    (synth, rev)
}

fn load_server(
    names_json: &str,
    x4_t: &str,
    x5_t: &str,
    x7_t: &str,
    hccl5_json: &str,
    hccl7_json: &str,
) -> ServerData {
    // 名称表：CID → 中文名 / 英文名。
    let names: HashMap<String, String> = serde_json::from_str(names_json).expect("valid JSON");
    let en_names: HashMap<String, String> =
        serde_json::from_str(include_str!("../assets/hcdata/cid_to_name_gjf_en.json"))
            .expect("valid JSON");
    let stars: HashMap<String, String> =
        serde_json::from_str(include_str!("../assets/hcdata/cid_to_star.json"))
            .expect("valid JSON");

    let mut cid_to_star = BTreeMap::new();
    let mut cid_to_name = BTreeMap::new();
    let mut cid_to_name_en = BTreeMap::new();
    for (cid, name) in &names {
        let cid: u32 = cid.parse().expect("numeric cid");
        cid_to_name.insert(cid, name.clone());
        if let Some(en) = en_names.get(&cid.to_string()) {
            cid_to_name_en.insert(cid, en.clone());
        }
        if let Some(star) = stars.get(&cid.to_string()) {
            cid_to_star.insert(cid, star.parse().expect("numeric star"));
        }
    }

    let (synth4, rev4) = build_synth(x4_t);
    let (synth5, rev5) = build_synth(x5_t);
    let (synth7, rev7) = build_synth(x7_t);

    let hccl5: HashMap<u32, HashSet<u32>> =
        serde_json::from_str::<HashMap<String, Vec<String>>>(hccl5_json)
            .expect("valid JSON")
            .into_iter()
            .map(|(k, v)| {
                (
                    k.parse::<u32>().expect("numeric cid"),
                    v.into_iter()
                        .map(|c| c.parse().expect("numeric cid"))
                        .collect(),
                )
            })
            .collect();
    let hccl7: HashMap<u32, HashSet<u32>> =
        serde_json::from_str::<HashMap<String, Vec<String>>>(hccl7_json)
            .expect("valid JSON")
            .into_iter()
            .map(|(k, v)| {
                (
                    k.parse::<u32>().expect("numeric cid"),
                    v.into_iter()
                        .map(|c| c.parse().expect("numeric cid"))
                        .collect(),
                )
            })
            .collect();

    ServerData {
        cid_to_star,
        cid_to_name,
        cid_to_name_en,
        synth4,
        synth5,
        synth7,
        rev4,
        rev5,
        rev7,
        hccl5,
        hccl7,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_all_servers() {
        let data = GameData::load();
        assert_eq!(data.global.cid_to_star.len(), 566);
        assert!(!data.global.synth4.is_empty());
        assert!(!data.global.synth5.is_empty());
        assert!(!data.global.synth7.is_empty());
        // 银河魅魔应为 7★。
        let cid = data
            .global
            .cid_to_name
            .iter()
            .find(|(_, n)| n.as_str() == "银河魅魔")
            .map(|(c, _)| *c)
            .expect("hero exists");
        assert_eq!(data.global.star(cid), 7);
    }

    #[test]
    fn rev_index_matches_synth() {
        let data = GameData::load();
        for (result, pairs) in &data.global.rev5 {
            for &(a, b) in pairs {
                assert_eq!(data.global.synth5[&(a, b)], *result);
            }
        }
    }
}
