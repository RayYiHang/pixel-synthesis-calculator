//! 合成路线搜索。
//!
//! 给定目标英雄与已有材料库存，枚举所有满足库存约束的合成路线：
//! - 4★ 目标：直接由持有的 3★ 两两合成；
//! - 5★ 目标：4★ 材料可来自库存或由两个 3★ 现场合成；
//! - 7★ 目标：5★ 材料可来自库存或由两个 4★ 合成（4★ 又可由 3★ 合成）。
//!
//! 中间合成结果受目标 HCCL（合法材料表）约束，与原版计算器行为一致。

use crate::model::ServerData;
use std::collections::{BTreeMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

/// 一条完整合成路线。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Route {
    /// 合成步骤，`(材料a, 材料b, 结果)`；首项为最终合成，其余为中间合成。
    pub steps: Vec<(u32, u32, u32)>,
    /// 消耗的原始材料（叶子节点）。
    pub mats: Vec<u32>,
}

/// 搜索结果。
#[derive(Debug, Default)]
pub struct SearchOutcome {
    pub routes: Vec<Route>,
    /// 因达到上限被截断。
    pub truncated: bool,
    /// 实际访问的搜索节点数。
    pub visits: u64,
}

/// 搜索规模上限，防止病态输入导致长时间占用 CPU。
pub struct SearchLimits {
    pub max_routes: usize,
    pub max_visits: u64,
}

impl Default for SearchLimits {
    fn default() -> Self {
        SearchLimits {
            max_routes: 100_000,
            max_visits: 100_000_000,
        }
    }
}

/// 库存：CID → 数量（仅含数量 > 0 的项）。
pub type Inventory = BTreeMap<u32, u32>;

/// 搜索器：深度优先回溯枚举全部可行路线。
struct Searcher<'a> {
    /// 中间合成是否受目标合法材料表约束。
    hccl: Option<&'a HashSet<u32>>,
    /// 4★ 结果 → 3★ 材料对（深度 1 的中间合成）。
    rev4: &'a BTreeMap<u32, Vec<(u32, u32)>>,
    /// 5★ 结果 → 4★ 材料对（深度 2 的中间合成）。
    rev5: &'a BTreeMap<u32, Vec<(u32, u32)>>,
    /// 是否允许虚拟叶子材料（全部配方模式）。
    allow_virtual: bool,
    limits: SearchLimits,
    visits: u64,
    truncated: bool,
    seen: HashSet<u64>,
}

impl<'a> Searcher<'a> {
    fn over_budget(&mut self) -> bool {
        if self.visits > self.limits.max_visits {
            self.truncated = true;
            return true;
        }
        false
    }

    /// 依次满足 `items` 中的每个需求项 `(cid, 可合成深度)`。
    ///
    /// 深度 0 表示只能使用库存；深度 d > 0 表示可用库存，或通过一次合成获得
    /// （合成材料以深度 d-1 满足）。满足全部项后记录一条路线。
    fn obtain(
        &mut self,
        items: &[(u32, u8)],
        fin: (u32, u32, u32),
        inv: &mut Inventory,
        steps: &mut Vec<(u32, u32, u32)>,
        mats: &mut Vec<u32>,
        out: &mut Vec<Route>,
    ) {
        self.visits += 1;
        if self.over_budget() || out.len() >= self.limits.max_routes {
            self.truncated = self.truncated || out.len() >= self.limits.max_routes;
            return;
        }
        let Some((&(cid, depth), rest)) = items.split_first() else {
            // 全部需求已满足：按规范化签名去重后记录。
            let mut sig = DefaultHasher::new();
            let mut sorted_steps = steps.clone();
            sorted_steps.sort();
            let mut sorted_mats = mats.clone();
            sorted_mats.sort();
            sorted_steps.hash(&mut sig);
            sorted_mats.hash(&mut sig);
            if self.seen.insert(sig.finish()) {
                let mut all_steps = Vec::with_capacity(steps.len() + 1);
                all_steps.push(fin);
                all_steps.extend(sorted_steps);
                out.push(Route {
                    steps: all_steps,
                    mats: sorted_mats,
                });
            }
            return;
        };

        // 方式一：直接消耗库存。
        let owned = inv.get(&cid).copied().unwrap_or(0);
        if owned > 0 {
            inv.insert(cid, owned - 1);
            mats.push(cid);
            self.obtain(rest, fin, inv, steps, mats, out);
            mats.pop();
            inv.insert(cid, owned);
        }

        // 方式二：虚拟叶子（仅全部配方模式）——深度耗尽时不再要求库存持有。
        if depth == 0 && self.allow_virtual {
            mats.push(cid);
            self.obtain(rest, fin, inv, steps, mats, out);
            mats.pop();
        }

        // 方式二：现场合成（受深度与合法材料表约束）。
        // 深度 2：合成 5★（4★+4★）；深度 1：合成 4★（3★+3★）。
        let rev = match depth {
            2 => Some(self.rev5),
            1 => Some(self.rev4),
            _ => None,
        };
        if let Some(rev) = rev.filter(|_| self.hccl.is_none_or(|s| s.contains(&cid))) {
            for &(a, b) in rev.get(&cid).into_iter().flatten() {
                let mut next = Vec::with_capacity(rest.len() + 2);
                next.push((a, depth - 1));
                next.push((b, depth - 1));
                next.extend_from_slice(rest);
                steps.push((a, b, cid));
                self.obtain(&next, fin, inv, steps, mats, out);
                steps.pop();
            }
        }
    }
}

/// 枚举目标英雄的全部可合成路线。路线按步骤数、材料数升序排序。
///
/// `allow_virtual` 为 `true` 时忽略库存约束（叶子材料视为虚拟可用），用于
/// 展示目标的全部理论配方；此时 `inventory` 仅作为混合来源参与枚举。
pub fn search_routes(
    data: &ServerData,
    target: u32,
    inventory: &Inventory,
    limits: SearchLimits,
    allow_virtual: bool,
) -> SearchOutcome {
    let star = data.star(target);
    let Some(rev) = data.recipes_for(star) else {
        return SearchOutcome::default();
    };
    let mut pairs: Vec<(u32, u32)> = rev
        .iter()
        .flat_map(|(result, ps)| ps.iter().map(move |&(a, b)| (a, b, *result)))
        .filter(|&(_, _, r)| r == target)
        .map(|(a, b, _)| (a, b))
        .collect();
    pairs.sort();

    let depth: u8 = match star {
        4 => 0,
        5 => 1,
        7 => 2,
        _ => return SearchOutcome::default(),
    };

    let mut inv = inventory.clone();
    inv.retain(|_, n| *n > 0);
    if !allow_virtual && inv.is_empty() {
        return SearchOutcome::default();
    }

    let mut searcher = Searcher {
        hccl: data.hccl_for(target, star),
        rev4: &data.rev4,
        rev5: &data.rev5,
        allow_virtual,
        limits,
        visits: 0,
        truncated: false,
        seen: HashSet::new(),
    };
    let mut out = Vec::new();
    for (a, b) in pairs {
        let items = [(a, depth), (b, depth)];
        let mut steps = Vec::new();
        let mut mats = Vec::new();
        searcher.obtain(
            &items,
            (a, b, target),
            &mut inv,
            &mut steps,
            &mut mats,
            &mut out,
        );
        if searcher.truncated && out.len() >= searcher.limits.max_routes {
            break;
        }
    }
    let visits = searcher.visits;
    let truncated = searcher.truncated;
    out.sort_by(|x, y| {
        (x.steps.len(), x.mats.len(), &x.steps, &x.mats).cmp(&(
            y.steps.len(),
            y.mats.len(),
            &y.steps,
            &y.mats,
        ))
    });
    SearchOutcome {
        routes: out,
        truncated,
        visits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::GameData;

    fn cid_by_name(data: &ServerData, name: &str) -> u32 {
        data.cid_to_name
            .iter()
            .find(|(_, n)| n.as_str() == name)
            .map(|(c, _)| *c)
            .unwrap_or_else(|| panic!("hero {name} not found"))
    }

    #[test]
    fn four_star_direct_pair() {
        let data = GameData::load();
        let d = &data.global;
        // 任取一个 4★ 目标，给齐一种 3★ 材料组合。
        let (&pair, &result) = d.synth4.iter().next().expect("has recipes");
        let (a, b) = pair;
        let inv = [(a, 1u32), (b, 1u32)].into_iter().collect::<Inventory>();
        let out = search_routes(d, result, &inv, SearchLimits::default(), false);
        assert!(out.routes.iter().any(|r| r.steps == vec![(a, b, result)]));
    }

    #[test]
    fn five_star_respects_inventory() {
        let data = GameData::load();
        let d = &data.global;
        let (&pair, &target) = d.synth5.iter().next().expect("has recipes");
        let (a, b) = pair;
        // 仅有 3★ 材料：a 必须可由两个 3★ 合成才可能有路线。
        let mut inv = Inventory::new();
        if let Some(pa) = d.rev4.get(&a) {
            let (x, y) = pa[0];
            inv.insert(x, 1);
            inv.insert(y, 1);
        }
        if let Some(pb) = d.rev4.get(&b) {
            let (x, y) = pb[0];
            *inv.entry(x).or_insert(0) += 1;
            *inv.entry(y).or_insert(0) += 1;
        }
        let out = search_routes(d, target, &inv, SearchLimits::default(), false);
        // 每条路线消耗的原始材料不超过库存。
        for route in &out.routes {
            let mut used: Inventory = Inventory::new();
            for m in &route.mats {
                *used.entry(*m).or_insert(0) += 1;
            }
            for (cid, n) in &used {
                assert!(
                    inv.get(cid).copied().unwrap_or(0) >= *n,
                    "route consumes more materials than owned"
                );
            }
            // 步骤链应闭合：中间结果恰好是最终材料。
            assert_eq!(route.steps[0].2, target);
        }
    }

    #[test]
    fn galaxy_demon_known_case() {
        // 原版示例：银河魅魔（7★）。
        let data = GameData::load();
        let d = &data.global;
        let target = cid_by_name(d, "银河魅魔");
        assert_eq!(d.star(target), 7);
        let hccl = d.hccl_for(target, 7).expect("hccl exists");
        // 选一条完全满足 HCCL 约束的 5★+5★ 配方链，并备齐全部 3★ 材料。
        let mut chain = None;
        for &(a, b) in d.rev7[&target].iter() {
            if !hccl.contains(&a) || !hccl.contains(&b) {
                continue;
            }
            let Some(pa) = d.rev5.get(&a).and_then(|v| {
                v.iter().find(|p| {
                    hccl.contains(&p.0)
                        && hccl.contains(&p.1)
                        && d.rev4.get(&p.0).is_some_and(|v| !v.is_empty())
                        && d.rev4.get(&p.1).is_some_and(|v| !v.is_empty())
                })
            }) else {
                continue;
            };
            let Some(pb) = d.rev5.get(&b).and_then(|v| {
                v.iter().find(|p| {
                    hccl.contains(&p.0)
                        && hccl.contains(&p.1)
                        && d.rev4.get(&p.0).is_some_and(|v| !v.is_empty())
                        && d.rev4.get(&p.1).is_some_and(|v| !v.is_empty())
                })
            }) else {
                continue;
            };
            chain = Some((a, pa, b, pb));
            break;
        }
        let (a, pa, b, pb) = chain.expect("a fully legal chain exists");
        let mut inv = Inventory::new();
        for side in [a, b] {
            let p4 = if side == a { pa } else { pb };
            for mid in [p4.0, p4.1] {
                let p3 = &d.rev4[&mid][0];
                *inv.entry(p3.0).or_insert(0) += 1;
                *inv.entry(p3.1).or_insert(0) += 1;
            }
        }
        let out = search_routes(d, target, &inv, SearchLimits::default(), false);
        assert!(
            !out.routes.is_empty(),
            "fully supplied chain must yield at least one route"
        );
        let best = &out.routes[0];
        assert_eq!(best.steps[0].2, target);
        // 7★ 路线 = 1 次最终合成 + 2 次 4★→5★ + 4 次 3★→4★。
        assert_eq!(best.steps.len(), 7, "7★ route has 7 synthesis steps");
        assert_eq!(best.mats.len(), 8, "7★ route consumes 8 three-star crews");
    }

    #[test]
    fn insufficient_materials_yields_nothing() {
        let data = GameData::load();
        let d = &data.global;
        let target = cid_by_name(d, "银河魅魔");
        let out = search_routes(d, target, &Inventory::new(), SearchLimits::default(), false);
        assert!(out.routes.is_empty());
    }

    #[test]
    fn virtual_mode_lists_all_recipes_without_inventory() {
        // 只选目标、无材料时：枚举全部理论配方。
        let data = GameData::load();
        let d = &data.global;
        let target = cid_by_name(d, "银河魅魔");
        let out = search_routes(d, target, &Inventory::new(), SearchLimits::default(), true);
        assert!(!out.routes.is_empty(), "virtual mode must list recipes");
        // 每条路线的步骤链闭合且不消耗任何库存。
        for route in &out.routes {
            assert_eq!(route.steps[0].2, target);
            assert_eq!(route.steps.len(), 7);
            assert_eq!(route.mats.len(), 8);
        }
        // 无 3★ 材料库存约束时，任何通过 HCCL 可达的最终配方对都应被覆盖。
        let hccl = d.hccl_for(target, 7).expect("hccl exists");
        let craftable_pair_exists = d.rev7[&target].iter().any(|&(a, b)| {
            hccl.contains(&a)
                && hccl.contains(&b)
                && d.rev5.get(&a).is_some_and(|v| {
                    v.iter().any(|p| {
                        hccl.contains(&p.0)
                            && hccl.contains(&p.1)
                            && d.rev4.contains_key(&p.0)
                            && d.rev4.contains_key(&p.1)
                    })
                })
                && d.rev5.get(&b).is_some_and(|v| {
                    v.iter().any(|p| {
                        hccl.contains(&p.0)
                            && hccl.contains(&p.1)
                            && d.rev4.contains_key(&p.0)
                            && d.rev4.contains_key(&p.1)
                    })
                })
        });
        if craftable_pair_exists {
            let covered = out.routes.iter().any(|r| {
                let (a, b, _) = r.steps[0];
                hccl.contains(&a) && hccl.contains(&b)
            });
            assert!(covered, "virtual mode must cover craftable final pairs");
        }
    }

    /// 性能基准：全部 3★ 材料各 2 份，搜索 7★ 路线。默认忽略，需手动运行：
    /// `cargo test --release -- --ignored --nocapture perf_full_materials`
    #[test]
    #[ignore]
    fn perf_full_materials() {
        let data = GameData::load();
        let d = &data.global;
        let target = cid_by_name(d, "银河魅魔");
        let mut inv = Inventory::new();
        for (cid, star) in &d.cid_to_star {
            if *star == 3 {
                inv.insert(*cid, 2);
            }
        }
        let started = std::time::Instant::now();
        let out = search_routes(d, target, &inv, SearchLimits::default(), false);
        let elapsed = started.elapsed();
        println!(
            "routes={} truncated={} visits={} elapsed={:.2}s",
            out.routes.len(),
            out.truncated,
            out.visits,
            elapsed.as_secs_f64()
        );
    }
}
