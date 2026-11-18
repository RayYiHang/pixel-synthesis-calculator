//! 主界面：目标选择、材料录入、路线结果展示。

use crate::model::{GameData, Server, ServerData};
use crate::search::{search_routes, Inventory, Route, SearchLimits, SearchOutcome};
use crate::update;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{
    button::{Button, ButtonVariants},
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    dialog::DialogFooter,
    input::{Input, InputEvent, InputState},
    label::Label,
    notification::Notification,
    pagination::Pagination,
    scroll::ScrollableElement as _,
    searchable_list::{SearchableListItem, SearchableVec},
    select::{Select, SelectEvent, SelectState},
    spinner::Spinner,
    ActiveTheme, Disableable as _, Icon, IndexPath, Sizable as _, Theme, ThemeMode, TitleBar,
    WindowExt,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    div, px, AnyElement, AnyWindowHandle, AppContext as _, Context, Entity, Focusable as _,
    FontWeight, Global, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    RenderOnce, SharedString, Styled as _, Subscription, Window,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// 结果列表每页显示条数。
const PAGE_SIZE: usize = 50;
/// 模糊匹配最低相似度（jaro-winkler）。
const FUZZY_MIN: f64 = 0.55;

/// 目标 / 材料 Combobox 的委托类型别名。
type HeroComboState = ComboboxState<SearchableVec<HeroItem>>;

/// 主窗口句柄，供后台任务完成后的窗口级操作（通知等）使用。
pub struct MainWindowHandle(pub AnyWindowHandle);
impl Global for MainWindowHandle {}

/// Combobox 中的英雄条目。
#[derive(Clone)]
struct HeroItem {
    cid: u32,
    name: SharedString,
    en: SharedString,
    star: u8,
}

impl SearchableListItem for HeroItem {
    type Value = u32;

    fn title(&self) -> SharedString {
        format!("{} {}★", self.name, self.star).into()
    }

    fn value(&self) -> &Self::Value {
        &self.cid
    }

    fn matches(&self, query: &str) -> bool {
        let q = query.trim();
        if q.is_empty() {
            return true;
        }
        let ql = q.to_lowercase();
        self.name.to_lowercase().contains(&ql)
            || self.en.to_lowercase().contains(&ql)
            || self.cid.to_string().contains(&ql)
            || format!("{}★", self.star) == ql
            || jaro(self.name.as_ref(), q) >= FUZZY_MIN
            || jaro(self.en.as_ref(), q) >= FUZZY_MIN
    }
}

fn jaro(a: &str, b: &str) -> f64 {
    strsim::jaro_winkler(a, b)
}

/// 星级小徽标（应用自有展示组件，取色全部来自主题 token）。
#[derive(IntoElement)]
struct StarBadge {
    star: u8,
}

impl RenderOnce for StarBadge {
    fn render(self, _: &mut Window, cx: &mut gpui_kit::App) -> impl IntoElement {
        let (bg, fg) = match self.star {
            7 => (cx.theme().primary, cx.theme().primary_foreground),
            5 => (cx.theme().accent, cx.theme().accent_foreground),
            4 => (cx.theme().secondary, cx.theme().secondary_foreground),
            _ => (cx.theme().muted, cx.theme().muted_foreground),
        };
        div()
            .px_1p5()
            .py_0p5()
            .rounded_sm()
            .bg(bg)
            .text_color(fg)
            .text_xs()
            .child(format!("{}★", self.star))
    }
}

/// 更新检测 / 安装状态。
#[derive(Clone)]
enum UpdateState {
    /// 尚未检测。
    Idle,
    /// 检测中。
    Checking,
    /// 已是最新。
    UpToDate,
    /// 发现新版本。
    Available { tag: String, url: String },
    /// 正在下载。
    Downloading,
    /// 安装结果提示（成功 / 失败原因）。
    Finished { ok: bool, message: String },
}

/// 收藏文件结构（持久化到用户主目录）。
#[derive(serde::Serialize, serde::Deserialize)]
struct FavoritesFile {
    routes: Vec<Route>,
}

/// 主窗口视图。
pub struct App {
    game: Arc<GameData>,
    data: Arc<ServerData>,
    server: Server,
    server_select: Entity<SelectState<SearchableVec<&'static str>>>,
    target_combo: Entity<HeroComboState>,
    mat_combo: Entity<HeroComboState>,
    paste_input: Entity<InputState>,
    account_input: Entity<InputState>,
    target_cid: Option<u32>,
    /// 材料库存：CID → 数量。库存是唯一事实来源，Combobox 选择仅作为输入通道。
    counts: Inventory,
    /// 材料行的展示顺序（最近的选择顺序）。
    mat_order: Vec<u32>,
    computing: bool,
    results: Option<Arc<SearchOutcome>>,
    result_target: Option<u32>,
    page: usize,
    /// 路线筛选：按英雄名搜索。
    route_filter_text: Entity<InputState>,
    filter_text: String,
    /// 路线筛选：必须用到所选材料之一（多选，任一命中即保留）。
    route_filter_mats: Entity<HeroComboState>,
    filter_cids: Vec<u32>,
    /// 筛选后保留的路线索引（None 表示未启用筛选）。
    filtered: Option<Arc<Vec<usize>>>,
    /// 收藏的路线（持久化到用户目录）。
    favorites: Vec<Route>,
    /// 是否正在浏览收藏夹。
    show_favorites: bool,
    status: SharedString,
    update_state: UpdateState,
    _subs: Vec<Subscription>,
}

impl App {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let game = Arc::new(GameData::load());
        let data = game.server(Server::Global);

        let server_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec!["国际服", "国服"]),
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let target_combo = cx
            .new(|cx| ComboboxState::new(target_items(&data), vec![], window, cx).searchable(true));
        let mat_combo = cx.new(|cx| {
            ComboboxState::new(mat_items(&data), vec![], window, cx)
                .searchable(true)
                .multiple(true)
        });
        let paste_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("粘贴材料名单，如：破败钢琴家，虫女士，3x")
        });
        let account_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("游戏内账号角色名"));
        let route_filter_text = cx
            .new(|cx| InputState::new(window, cx).placeholder("按英雄名筛选路线（中文名 / 编号）"));
        let route_filter_mats = cx.new(|cx| {
            ComboboxState::new(mat_items(&data), vec![], window, cx)
                .searchable(true)
                .multiple(true)
        });

        let mut _subs = Vec::new();
        _subs.push(Self::subscribe_target(&target_combo, window, cx));
        _subs.push(Self::subscribe_materials(&mat_combo, window, cx));
        _subs.push(
            cx.subscribe_in(&server_select, window, |this, _, event, window, cx| {
                if let SelectEvent::Confirm(Some(label)) = event {
                    let server = match *label {
                        "国服" => Server::China,
                        _ => Server::Global,
                    };
                    if server != this.server {
                        this.switch_server(server, window, cx);
                    }
                }
            }),
        );
        _subs.push(
            cx.subscribe_in(&paste_input, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.import_pasted_materials(window, cx);
                }
            }),
        );
        _subs.extend(Self::subscribe_route_filters(
            &route_filter_text,
            &route_filter_mats,
            window,
            cx,
        ));

        Self {
            status: "就绪 · 数据已加载".into(),
            game,
            data,
            server: Server::Global,
            server_select,
            target_combo,
            mat_combo,
            paste_input,
            account_input,
            target_cid: None,
            counts: Inventory::new(),
            mat_order: Vec::new(),
            computing: false,
            results: None,
            result_target: None,
            page: 1,
            route_filter_text,
            filter_text: String::new(),
            route_filter_mats,
            filter_cids: Vec::new(),
            filtered: None,
            favorites: Self::load_favorites(),
            show_favorites: false,
            update_state: UpdateState::Idle,
            _subs,
        }
        .with_startup_update_check(cx)
    }

    /// 路线筛选控件的订阅：文本与材料任一变化即重新过滤。
    fn subscribe_route_filters(
        text_input: &Entity<InputState>,
        mats_combo: &Entity<HeroComboState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        vec![
            cx.subscribe_in(text_input, window, |this, _, event, _, cx| {
                if let InputEvent::Change = event {
                    this.filter_text = this.route_filter_text.read(cx).value().to_lowercase();
                    this.rebuild_filter();
                    cx.notify();
                }
            }),
            cx.subscribe_in(mats_combo, window, |this, _, event, _, cx| {
                let values = match event {
                    ComboboxEvent::Change(values) => values,
                    ComboboxEvent::Confirm(values) => values,
                };
                this.filter_cids = values.clone();
                this.rebuild_filter();
                cx.notify();
            }),
        ]
    }

    /// 路线筛选是否启用。
    fn filter_active(&self) -> bool {
        !self.filter_text.is_empty() || !self.filter_cids.is_empty()
    }

    /// 根据当前筛选条件重建保留的路线索引。
    fn rebuild_filter(&mut self) {
        if !self.filter_active() {
            self.filtered = None;
            return;
        }
        let Some(res) = &self.results else {
            self.filtered = None;
            return;
        };
        // 小写名称缓存（数量有限，全量构建）。
        let zh: HashMap<u32, String> = self
            .data
            .cid_to_name
            .iter()
            .map(|(c, n)| (*c, n.to_lowercase()))
            .collect();
        let en: HashMap<u32, String> = self
            .data
            .cid_to_name_en
            .iter()
            .map(|(c, n)| (*c, n.to_lowercase()))
            .collect();
        let text = self.filter_text.clone();
        let cids = &self.filter_cids;
        let mut indices = Vec::new();
        for (i, route) in res.routes.iter().enumerate() {
            if route_hits(route, &text, cids, &zh, &en) {
                indices.push(i);
            }
        }
        self.filtered = Some(Arc::new(indices));
        self.page = 1;
    }

    /// 启动时自动检测更新。
    fn with_startup_update_check(self, cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| {
                this.check_for_updates(cx);
            });
        })
        .detach();
        self
    }

    fn subscribe_target(
        entity: &Entity<HeroComboState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(entity, window, |this, _, event, _, cx| {
            let values = match event {
                ComboboxEvent::Change(values) => values,
                ComboboxEvent::Confirm(values) => values,
            };
            let new_target = values.last().copied();
            if this.target_cid != new_target {
                this.target_cid = new_target;
                this.results = None;
                this.result_target = None;
                this.page = 1;
                cx.notify();
            }
        })
    }

    fn subscribe_materials(
        entity: &Entity<HeroComboState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(entity, window, |this, _, event, _, cx| {
            // 勾选即加入库存（数量默认 1）；仅在选择确认（下拉关闭）时移除。
            // 注意：程序化 set_selected_values 不会发出事件，库存由本视图自行维护。
            match event {
                ComboboxEvent::Change(values) => {
                    for cid in values {
                        this.counts.entry(*cid).or_insert(1);
                    }
                    this.mat_order = values.clone();
                    cx.notify();
                }
                ComboboxEvent::Confirm(values) => {
                    let selected = values.clone();
                    this.counts.retain(|cid, _| selected.contains(cid));
                    this.mat_order = selected;
                    cx.notify();
                }
            }
        })
    }

    // ---------- 状态维护 ----------

    fn switch_server(&mut self, server: Server, window: &mut Window, cx: &mut Context<Self>) {
        self.server = server;
        self.data = self.game.server(server);
        self.results = None;
        self.result_target = None;
        self.filtered = None;
        self.page = 1;
        let target_cid = self.target_cid;
        let filter_cids = self.filter_cids.clone();
        let data = self.data.clone();
        let target_items = target_items(&data);
        let mat_items = mat_items(&data);
        self.target_combo = cx.new(|cx| {
            let mut state = ComboboxState::new(target_items, vec![], window, cx).searchable(true);
            if let Some(cid) = target_cid {
                state.set_selected_values(&[cid], window, cx);
            }
            state
        });
        self.mat_combo = cx.new(|cx| {
            ComboboxState::new(mat_items.clone(), vec![], window, cx)
                .searchable(true)
                .multiple(true)
        });
        self.route_filter_mats = cx.new(|cx| {
            ComboboxState::new(mat_items, vec![], window, cx)
                .searchable(true)
                .multiple(true)
        });
        let target_combo = self.target_combo.clone();
        let mat_combo = self.mat_combo.clone();
        let route_filter_mats = self.route_filter_mats.clone();
        self._subs
            .push(Self::subscribe_target(&target_combo, window, cx));
        self._subs
            .push(Self::subscribe_materials(&mat_combo, window, cx));
        self._subs.extend(Self::subscribe_route_filters(
            &self.route_filter_text.clone(),
            &route_filter_mats,
            window,
            cx,
        ));
        // 用已有库存恢复新 Combobox 的选择（不改变计数）。
        let cids: Vec<u32> = self.counts.keys().copied().collect();
        self.mat_combo.update(cx, |state, cx| {
            state.set_selected_values(&cids, window, cx);
        });
        // 恢复路线筛选的材料选择。
        self.route_filter_mats.update(cx, |state, cx| {
            state.set_selected_values(&filter_cids, window, cx);
        });
        self.status = format!("已切换到 {} 数据", server.label()).into();
        cx.notify();
    }

    fn clear_materials(&mut self, cx: &mut Context<Self>) {
        self.counts.clear();
        self.mat_order.clear();
        self.mat_combo
            .update(cx, |state, cx| state.clear_selection(cx));
        self.status = "已清空材料".into();
        cx.notify();
    }

    fn remove_material(&mut self, cid: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.counts.remove(&cid);
        self.mat_order.retain(|&c| c != cid);
        let cids: Vec<u32> = self.counts.keys().copied().collect();
        self.mat_combo.update(cx, |state, cx| {
            state.set_selected_values(&cids, window, cx);
        });
        cx.notify();
    }

    fn bump_material(&mut self, cid: u32, delta: i32, cx: &mut Context<Self>) {
        let current = self.counts.get(&cid).copied().unwrap_or(1) as i32;
        let next = (current + delta).clamp(1, 99);
        self.counts.insert(cid, next as u32);
        cx.notify();
    }

    // ---------- 材料导入 ----------

    /// 从粘贴文本导入材料名单，支持 `×N` / `xN` / `*N` / `N×` 数量写法。
    fn import_pasted_materials(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.paste_input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let (added, missing) = self.parse_material_names(&text);
        if added.is_empty() {
            window.push_notification(
                Notification::warning(format!("未识别到任何材料：{}", missing.join("、"))),
                cx,
            );
            return;
        }
        for (cid, n) in &added {
            *self.counts.entry(*cid).or_insert(0) += *n;
        }
        // 同步 Combobox 选择并刷新行顺序。
        let cids: Vec<u32> = self.counts.keys().copied().collect();
        self.mat_order = cids.clone();
        self.mat_combo.update(cx, |state, cx| {
            state.set_selected_values(&cids, window, cx);
        });
        let missing_note = if missing.is_empty() {
            String::new()
        } else {
            format!("，{} 项未识别", missing.len())
        };
        self.paste_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        window.push_notification(
            Notification::success(format!(
                "已添加材料，当前共 {} 种{}",
                self.counts.len(),
                missing_note
            )),
            cx,
        );
        cx.notify();
    }

    fn parse_material_names(&self, text: &str) -> (Vec<(u32, u32)>, Vec<String>) {
        let mut counts: Inventory = Inventory::new();
        let mut missing = Vec::new();
        let cleaned = text
            .replace(['，', '；', '、', '\n', '\r', ';', ','], ",")
            .replace(['[', ']'], "");
        for raw in cleaned.split(',') {
            let item = raw.trim().trim_matches(['\'', '"', ' ']);
            if item.is_empty() {
                continue;
            }
            let (name, mult) = split_multiplier(item);
            let Some(cid) = self.fuzzy_find(name) else {
                missing.push(name.to_string());
                continue;
            };
            *counts.entry(cid).or_insert(0) += mult.max(1);
        }
        (counts.into_iter().collect(), missing)
    }

    /// 按名称模糊查找英雄（中英文名，jaro-winkler）。
    fn fuzzy_find(&self, name: &str) -> Option<u32> {
        if name.is_empty() {
            return None;
        }
        let mut best: Option<(f64, u32)> = None;
        for (cid, cname) in &self.data.cid_to_name {
            let s = jaro(cname, name);
            if s >= FUZZY_MIN && best.is_none_or(|(bs, _)| s > bs) {
                best = Some((s, *cid));
            }
        }
        for (cid, cname) in &self.data.cid_to_name_en {
            let s = jaro(cname, name);
            if s >= FUZZY_MIN && best.is_none_or(|(bs, _)| s > bs) {
                best = Some((s, *cid));
            }
        }
        best.map(|(_, cid)| cid)
    }

    /// 从游戏 API 拉取账号船员名单（测试功能，与原版行为一致）。
    fn import_from_account(&mut self, cx: &mut Context<Self>) {
        let username = self.account_input.read(cx).value().trim().to_string();
        if username.is_empty() {
            return;
        }
        self.status = format!("正在从服务器拉取「{username}」的船员名单…").into();
        cx.notify();
        let url = format!(
            "https://api.pixelstarships.com/PublicService/GetShipCharactersByUsername?username={username}&accessToken=3211e26e-6ed7-4da0-bbab-a1c8b34af88f"
        );
        let data = self.data.clone();
        let mat_combo = self.mat_combo.clone();
        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_executor()
                .spawn(async move { fetch_account_crew(&url, &data) })
                .await;
            let handle = cx.read_global::<MainWindowHandle, _>(|g, _| g.0);
            let _ = cx.update_window(handle, move |_, window, cx| match fetched {
                Ok(ids) => {
                    let count = ids.len();
                    this.update(cx, |this, cx| {
                        for cid in &ids {
                            *this.counts.entry(*cid).or_insert(0) += 1;
                        }
                        this.mat_order = this.counts.keys().copied().collect();
                        this.status =
                            format!("已导入 {count} 种可用材料（账号：{username}）").into();
                        cx.notify();
                    })
                    .ok();
                    let cids: Vec<u32> = this
                        .upgrade()
                        .map(|entity| entity.read(cx).counts.keys().copied().collect::<Vec<u32>>())
                        .unwrap_or_default();
                    mat_combo.update(cx, |state, cx| {
                        state.set_selected_values(&cids, window, cx);
                    });
                }
                Err(msg) => {
                    this.update(cx, |this, cx| {
                        this.status = "就绪".into();
                        cx.notify();
                    })
                    .ok();
                    window.push_notification(Notification::error(msg), cx);
                }
            });
        })
        .detach();
    }

    // ---------- 计算 ----------

    fn compute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.target_cid else {
            window.push_notification(Notification::warning("请先选择目标英雄"), cx);
            return;
        };
        let star = self.data.star(target);
        if ![4, 5, 7].contains(&star) {
            window.push_notification(
                Notification::warning("仅支持选择 4★、5★、7★ 英雄作为合成目标"),
                cx,
            );
            return;
        }
        let inv = self.counts.clone();
        let all_recipes = self.counts.is_empty();
        let data = self.data.clone();
        self.computing = true;
        self.status = if all_recipes {
            "未录入材料，正在枚举全部理论配方…".into()
        } else {
            "正在计算合成路线…".into()
        };
        cx.notify();
        let started = std::time::Instant::now();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    search_routes(&data, target, &inv, SearchLimits::default(), all_recipes)
                })
                .await;
            let elapsed = started.elapsed();
            let _ = this.update(cx, |this, cx| {
                this.computing = false;
                this.page = 1;
                this.result_target = Some(target);
                let (n, truncated, visits) =
                    (outcome.routes.len(), outcome.truncated, outcome.visits);
                this.results = Some(Arc::new(outcome));
                this.rebuild_filter();
                this.status = format!(
                    "共 {} 条可合成路线{} · 耗时 {:.1} ms · 搜索 {} 节点{}",
                    fmt_num(n),
                    if all_recipes {
                        "（全部理论配方）"
                    } else {
                        ""
                    },
                    elapsed.as_secs_f64() * 1000.0,
                    fmt_num(visits as usize),
                    if truncated { " · 结果已截断" } else { "" }
                )
                .into();
                cx.notify();
            });
        })
        .detach();
    }

    // ---------- 更新 ----------

    /// 检测 GitHub 最新 Release。
    fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        self.update_state = UpdateState::Checking;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let latest = cx
                .background_executor()
                .spawn(async move { update::fetch_latest() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.update_state = match latest {
                    Ok(rel) if update::is_newer(&rel.tag, update::CURRENT_VERSION) => {
                        UpdateState::Available {
                            tag: rel.tag,
                            url: rel.asset_url,
                        }
                    }
                    Ok(_) => UpdateState::UpToDate,
                    Err(msg) => UpdateState::Finished {
                        ok: false,
                        message: msg,
                    },
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// 一键更新：下载当前平台资产并安装。
    fn perform_update(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Available { url, .. } = &self.update_state else {
            return;
        };
        let url = url.clone();
        self.update_state = UpdateState::Downloading;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let work = cx.background_executor().spawn(async move {
                let ext = match std::env::consts::OS {
                    "windows" => "zip",
                    "macos" => "dmg",
                    _ => "deb",
                };
                let dest =
                    std::env::temp_dir().join(format!("{}-update.{ext}", env!("CARGO_PKG_NAME")));
                let _ = std::fs::remove_file(&dest);
                let _ = update::download_to(&url, &dest)?;
                let message = match update::install_auto(&dest)? {
                    update::InstallOutcome::Restart(new_exe) => {
                        // 启动新版本并退出当前实例。
                        std::process::Command::new(new_exe).spawn().ok();
                        std::process::exit(0);
                    }
                    update::InstallOutcome::Message(message) => message,
                };
                Ok::<String, String>(message)
            });
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.update_state = match result {
                    Ok(message) => UpdateState::Finished { ok: true, message },
                    Err(message) => UpdateState::Finished { ok: false, message },
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// 状态栏右下角的更新控件。
    fn render_update_control(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let version_label = Label::new(SharedString::from(format!("v{}", update::CURRENT_VERSION)))
            .text_xs()
            .text_color(cx.theme().muted_foreground);
        let control: AnyElement = match &self.update_state {
            UpdateState::Idle => Button::new("check-update")
                .ghost()
                .xsmall()
                .label("检查更新")
                .icon(IconName::RefreshCw)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.check_for_updates(cx);
                }))
                .into_any_element(),
            UpdateState::Checking => h_flex()
                .gap_1()
                .items_center()
                .child(Spinner::new().xsmall())
                .child(
                    Label::new("检查更新中…")
                        .text_xs()
                        .text_color(cx.theme().muted_foreground),
                )
                .into_any_element(),
            UpdateState::UpToDate => Label::new("已是最新版本")
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .into_any_element(),
            UpdateState::Available { tag, .. } => Button::new("do-update")
                .primary()
                .xsmall()
                .label(SharedString::from(format!("一键更新到 {tag}")))
                .icon(IconName::Download)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.perform_update(cx);
                }))
                .into_any_element(),
            UpdateState::Downloading => h_flex()
                .gap_1()
                .items_center()
                .child(Spinner::new().xsmall())
                .child(
                    Label::new("正在下载更新…")
                        .text_xs()
                        .text_color(cx.theme().muted_foreground),
                )
                .into_any_element(),
            UpdateState::Finished { ok, message } => h_flex()
                .gap_2()
                .items_center()
                .child(
                    Icon::new(if *ok {
                        IconName::CircleCheck
                    } else {
                        IconName::CircleAlert
                    })
                    .xsmall()
                    .text_color(if *ok {
                        cx.theme().success
                    } else {
                        cx.theme().danger
                    }),
                )
                .child(
                    Label::new(SharedString::from(message.clone()))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .max_w(px(280.))
                        .truncate(),
                )
                .child(
                    Button::new("recheck-update")
                        .ghost()
                        .xsmall()
                        .label(if *ok { "检查更新" } else { "重试" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.check_for_updates(cx);
                        })),
                )
                .into_any_element(),
        };
        h_flex()
            .gap_2()
            .items_center()
            .child(control)
            .child(version_label)
    }

    // ---------- 收藏夹 ----------

    /// 收藏文件位置：用户主目录下。
    fn favorites_path() -> Option<std::path::PathBuf> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .ok()?;
        Some(
            std::path::PathBuf::from(home)
                .join(".pixel-synthesis-calculator")
                .join("favorites.json"),
        )
    }

    fn load_favorites() -> Vec<Route> {
        Self::favorites_path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<FavoritesFile>(&bytes).ok())
            .map(|file| file.routes)
            .unwrap_or_default()
    }

    fn save_favorites(&self) {
        let Some(path) = Self::favorites_path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&FavoritesFile {
            routes: self.favorites.clone(),
        }) {
            let _ = std::fs::write(path, json);
        }
    }

    fn is_favorite(&self, route: &Route) -> bool {
        self.favorites.contains(route)
    }

    fn toggle_favorite(&mut self, route: &Route, cx: &mut Context<Self>) {
        if let Some(pos) = self.favorites.iter().position(|r| r == route) {
            self.favorites.remove(pos);
        } else {
            self.favorites.insert(0, route.clone());
        }
        self.save_favorites();
        cx.notify();
    }

    /// 收藏夹视图。
    fn render_favorites(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.favorites.is_empty() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Star)
                        .large()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(Label::new("还没有收藏的路线").font_weight(FontWeight::MEDIUM))
                .child(
                    Label::new("在计算结果中点击路线右侧的星标即可收藏，收藏会跨会话保留。")
                        .text_sm()
                        .text_color(cx.theme().muted_foreground),
                )
                .into_any_element();
        }
        let total = self.favorites.len();
        let total_pages = total.div_ceil(PAGE_SIZE).max(1);
        let page = self.page.clamp(1, total_pages);
        let start = (page - 1) * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(total);
        let slice: Vec<Route> = self.favorites[start..end].to_vec();
        let items = slice
            .iter()
            .enumerate()
            .map(|(i, route)| self.render_route(start + i + 1, route, cx))
            .collect::<Vec<_>>();
        div()
            .id("favorites-list")
            .size_full()
            .overflow_y_scrollbar()
            .child(v_flex().px_4().py_2().children(items))
            .into_any_element()
    }

    // ---------- 渲染 ----------

    fn render_title_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = cx.theme().mode;
        TitleBar::new()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Label::new("像素星舰 · 英雄合成计算器")
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM),
                    )
                    .child(
                        Label::new("3★+3★→4★ · 4★+4★→5★ · 5★+5★→7★")
                            .text_xs()
                            .text_color(cx.theme().muted_foreground),
                    ),
            )
            .child(
                h_flex().gap_1().items_center().justify_end().child(
                    Button::new("toggle-theme")
                        .ghost()
                        .small()
                        .icon(if mode == ThemeMode::Dark {
                            IconName::Sun
                        } else {
                            IconName::Moon
                        })
                        .tooltip("切换深浅色主题")
                        .on_click(|_, _, cx| {
                            let next = if cx.theme().mode == ThemeMode::Dark {
                                ThemeMode::Light
                            } else {
                                ThemeMode::Dark
                            };
                            Theme::change(next, None, cx);
                        }),
                ),
            )
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let computing = self.computing;
        let has_results = self.results.as_ref().is_some_and(|r| !r.routes.is_empty());
        let star = self.target_cid.map(|t| self.data.star(t)).unwrap_or(0);
        let mat_rows = self.render_mat_rows(cx);

        v_flex()
            .w(px(340.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scrollbar()
            .gap_4()
            .p_4()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        Label::new("服务器")
                            .text_sm()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(Select::new(&self.server_select).w_full()),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        Label::new("目标英雄")
                            .text_sm()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Combobox::new(&self.target_combo)
                                    .placeholder("搜索 4★ / 5★ / 7★ 英雄…")
                                    .search_placeholder("中文名、英文名或编号")
                                    .highlight_matches(true)
                                    .cleanable(true)
                                    .w_full(),
                            )
                            .when(star > 0, |this| this.child(StarBadge { star })),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                Label::new("已有材料")
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .child(
                                Button::new("clear-materials")
                                    .ghost()
                                    .xsmall()
                                    .label("清空")
                                    .icon(IconName::X)
                                    .disabled(self.counts.is_empty())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.clear_materials(cx);
                                    })),
                            ),
                    )
                    .child(
                        Combobox::new(&self.mat_combo)
                            .placeholder("添加材料英雄（可多选、搜索）…")
                            .search_placeholder("中文名、英文名或编号")
                            .highlight_matches(true)
                            .cleanable(true)
                            .w_full(),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Input::new(&self.paste_input)
                                    .small()
                                    .cleanable(true)
                                    .w_full(),
                            )
                            .child(
                                Button::new("import-paste")
                                    .outline()
                                    .small()
                                    .label("导入")
                                    .disabled(computing)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import_pasted_materials(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        Button::new("import-account")
                            .ghost()
                            .small()
                            .label("从账号导入…")
                            .icon(IconName::Globe)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_account_dialog(window, cx);
                            })),
                    )
                    .child(mat_rows),
            )
            .child(
                v_flex()
                    .gap_2()
                    .mt_auto()
                    .child(
                        Button::new("compute")
                            .primary()
                            .large()
                            .label(if computing {
                                "计算中…"
                            } else {
                                "计算合成路线"
                            })
                            .icon(IconName::Search)
                            .loading(computing)
                            .disabled(computing)
                            .w_full()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.compute(window, cx);
                            })),
                    )
                    .child(
                        Button::new("export-csv")
                            .outline()
                            .label("导出 CSV")
                            .icon(IconName::Download)
                            .disabled(!has_results)
                            .w_full()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.export_csv(window, cx);
                            })),
                    ),
            )
    }

    fn render_mat_rows(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let order: Vec<u32> = self
            .mat_order
            .iter()
            .copied()
            .filter(|cid| self.counts.contains_key(cid))
            .collect();
        let mut rest: Vec<u32> = self
            .counts
            .keys()
            .copied()
            .filter(|cid| !order.contains(cid))
            .collect();
        rest.sort();
        let rows = order
            .into_iter()
            .chain(rest)
            .map(|cid| {
                let count = self.counts.get(&cid).copied().unwrap_or(1);
                let name: SharedString = self.data.name(cid).to_owned().into();
                let star = self.data.star(cid);
                v_flex()
                    .id(("mat-row", cid as usize))
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .border_1()
                    .border_color(cx.theme().border)
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .min_w_0()
                                    .child(StarBadge { star })
                                    .child(div().text_sm().min_w_0().truncate().child(name)),
                            )
                            .child(
                                Button::new(("mat-remove", cid as usize))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::X)
                                    .tooltip("移除材料")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.remove_material(cid, window, cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new(("mat-dec", cid as usize))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Minus)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.bump_material(cid, -1, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .min_w_6()
                                    .text_center()
                                    .child(SharedString::from(fmt_num(count as usize))),
                            )
                            .child(
                                Button::new(("mat-inc", cid as usize))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Plus)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.bump_material(cid, 1, cx);
                                    })),
                            )
                            .child(
                                Label::new("份")
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground),
                            ),
                    )
            })
            .collect::<Vec<_>>();
        v_flex().gap_1().children(rows)
    }

    fn render_results(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let computing = self.computing;
        let star = self.target_cid.map(|t| self.data.star(t)).unwrap_or(0);
        let has_target = self.target_cid.is_some();
        let target_name = self.target_cid.map(|t| self.data.name(t).to_string());
        let header = self.render_results_header(cx).into_any_element();
        let filter_bar = self.render_filter_bar(cx).into_any_element();
        let body = if self.show_favorites {
            self.render_favorites(cx).into_any_element()
        } else if computing {
            self.render_computing(cx).into_any_element()
        } else {
            let has_routes = self.results.as_ref().is_some_and(|r| !r.routes.is_empty());
            if has_routes {
                if self.filter_active() && self.filtered.as_ref().is_none_or(|v| v.is_empty()) {
                    self.render_filter_empty(cx).into_any_element()
                } else {
                    let (routes, offset) = self.page_slice().unwrap_or_default();
                    self.render_route_list(&routes, offset, cx)
                        .into_any_element()
                }
            } else {
                self.render_empty(target_name.as_deref(), star, has_target, cx)
                    .into_any_element()
            }
        };
        let pagination = self.render_pagination(cx).into_any_element();
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(cx.theme().background)
            .child(header)
            .when(!self.show_favorites, |this| this.child(filter_bar))
            .child(div().flex_1().min_h_0().child(body))
            .child(pagination)
    }

    /// 无筛选命中时的占位视图。
    fn render_filter_empty(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                Icon::new(IconName::SearchX)
                    .large()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(Label::new("没有符合筛选条件的路线").font_weight(FontWeight::MEDIUM))
            .child(
                Label::new("调整搜索关键词或材料筛选，或点击「清除筛选」查看全部路线。")
                    .text_sm()
                    .text_color(cx.theme().muted_foreground),
            )
    }

    /// 结果区顶部的筛选工具栏。
    fn render_filter_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_filter = self.filter_active();
        h_flex()
            .gap_2()
            .items_center()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                Input::new(&self.route_filter_text)
                    .small()
                    .prefix(Icon::new(IconName::Search).small())
                    .cleanable(true)
                    .w(px(220.)),
            )
            .child(
                Combobox::new(&self.route_filter_mats)
                    .placeholder("只看用到某些材料的路线…")
                    .search_placeholder("中文名、英文名或编号")
                    .highlight_matches(true)
                    .w(px(260.)),
            )
            .when(has_filter, |this| {
                this.child(
                    Label::new("筛选中")
                        .text_xs()
                        .text_color(cx.theme().primary),
                )
                .child(
                    Button::new("clear-route-filter")
                        .outline()
                        .xsmall()
                        .label("清除筛选")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.clear_route_filter(window, cx);
                        })),
                )
            })
            .child(
                Label::new("材料筛选：路线需用到所选英雄之一")
                    .text_xs()
                    .text_color(cx.theme().muted_foreground),
            )
    }

    /// 清除全部路线筛选。
    fn clear_route_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_text.clear();
        self.filter_cids.clear();
        self.route_filter_text
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.route_filter_mats
            .update(cx, |state, cx| state.clear_selection(cx));
        self.filtered = None;
        self.page = 1;
        cx.notify();
    }

    fn render_results_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let total = if self.show_favorites {
            self.favorites.len()
        } else {
            self.results.as_ref().map_or(0, |r| r.routes.len())
        };
        let truncated = self.results.as_ref().is_some_and(|r| r.truncated);
        let visible = if self.show_favorites {
            self.favorites.len()
        } else {
            self.visible_route_count()
        };
        let title = if self.show_favorites {
            "收藏夹"
        } else {
            "可合成路线"
        };
        let fav_count = self.favorites.len();
        let show_favorites = self.show_favorites;
        let target = self
            .result_target
            .or(self.target_cid)
            .map(|t| (self.data.name(t).to_string(), self.data.star(t)));
        h_flex()
            .justify_between()
            .items_center()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Label::new(title).font_weight(FontWeight::MEDIUM))
                    .when_some(
                        if show_favorites { None } else { target },
                        |this, (name, star)| {
                            this.child(StarBadge { star }).child(
                                Label::new(SharedString::from(name))
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground),
                            )
                        },
                    )
                    .when(visible > 0, |this| {
                        this.child(
                            Label::new(SharedString::from(
                                if !show_favorites && self.filter_active() {
                                    format!("筛选后 {} / {} 条", fmt_num(visible), fmt_num(total))
                                } else {
                                    format!("共 {} 条", fmt_num(total))
                                },
                            ))
                            .text_sm()
                            .text_color(cx.theme().muted_foreground),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .when(truncated && !show_favorites, |this| {
                        this.child(
                            Label::new("结果数量已达上限，仅显示部分结果")
                                .text_sm()
                                .text_color(cx.theme().warning),
                        )
                    })
                    .child(
                        Button::new("toggle-favorites")
                            .small()
                            .icon(if show_favorites {
                                IconName::StarFill
                            } else {
                                IconName::Star
                            })
                            .label(SharedString::from(fmt_num(fav_count)))
                            .tooltip("收藏夹：查看已收藏的合成路线")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_favorites = !this.show_favorites;
                                this.page = 1;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_computing(&mut self, _cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(Spinner::new().small())
            .child(Label::new("正在枚举全部可行合成路线…"))
    }

    fn render_empty(
        &mut self,
        target_name: Option<&str>,
        star: u8,
        has_target: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (title, detail) = if !has_target {
            (
                "尚未选择目标英雄",
                "在左侧搜索并选择 4★、5★ 或 7★ 目标，录入材料后点击「计算合成路线」。".to_string(),
            )
        } else if self.results.is_some() {
            (
                "没有找到可行的合成路线",
                "当前材料不足以合成该目标，下方列出这些材料可以直接合成的内容。".to_string(),
            )
        } else if star == 6 {
            (
                "该船员无法通过合成获得",
                "6★ 船员只能通过游戏内其他途径获取。".to_string(),
            )
        } else {
            (
                "选择目标英雄",
                target_name
                    .map(|n| format!("当前目标：{n}"))
                    .unwrap_or_default(),
            )
        };
        // 失败时展示「当前材料可以直接合成」的预览。
        let hints = if self.results.is_some() && !self.counts.is_empty() {
            self.craftable_previews()
        } else {
            Vec::new()
        };
        v_flex()
            .size_full()
            .overflow_y_scrollbar()
            .items_start()
            .justify_center()
            .gap_2()
            .px_8()
            .py_6()
            .child(
                v_flex()
                    .items_center()
                    .gap_2()
                    .mt_4()
                    .child(
                        Icon::new(IconName::PackageSearch)
                            .large()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(Label::new(title).text_lg().font_weight(FontWeight::MEDIUM))
                    .child(
                        Label::new(detail)
                            .text_sm()
                            .text_color(cx.theme().muted_foreground),
                    ),
            )
            .when(!hints.is_empty(), |this| {
                this.child(
                    v_flex()
                        .mt_4()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    Label::new("你的材料可以直接合成")
                                        .font_weight(FontWeight::MEDIUM),
                                )
                                .child(
                                    Label::new(SharedString::from(format!(
                                        "{} 项",
                                        fmt_num(hints.len())
                                    )))
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground),
                                ),
                        )
                        .children(hints.iter().take(12).map(|(star, a, b, result)| {
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(StarBadge { star: *star })
                                .child(
                                    Label::new(SharedString::from(format!(
                                        "{} + {} = {}",
                                        self.data.name(*a),
                                        self.data.name(*b),
                                        self.data.name(*result),
                                    )))
                                    .text_sm(),
                                )
                        }))
                        .when(hints.len() > 12, |this| {
                            this.child(
                                Label::new(SharedString::from(format!(
                                    "… 其余 {} 项从略",
                                    hints.len() - 12
                                )))
                                .text_xs()
                                .text_color(cx.theme().muted_foreground),
                            )
                        })
                        .child(
                            Label::new("预览仅统计一步可合成的结果，未考虑材料重复占用；补充材料后点击「计算合成路线」可查看完整路线。")
                                .text_xs()
                                .text_color(cx.theme().muted_foreground),
                        ),
                )
            })
    }

    /// 用当前库存一步可合成的内容预览：`(结果星级, a, b, 结果)`。
    ///
    /// 中间体可由库存直接提供，或由更低星级材料现场合成；仅做提示用途，
    /// 不精确扣减跨步骤的材料占用。
    fn craftable_previews(&self) -> Vec<(u8, u32, u32, u32)> {
        let d = &self.data;
        let inv = &self.counts;
        let owned = |c: u32| inv.get(&c).copied().unwrap_or(0);

        // 4★：库存 3★ 两两合成。
        let mut previews = Vec::new();
        let mut crafted4: HashSet<u32> = HashSet::new();
        for (&result, pairs) in &d.rev4 {
            if let Some(&(a, b)) = pairs.iter().find(|&&(a, b)| {
                if a == b {
                    owned(a) >= 2
                } else {
                    owned(a) >= 1 && owned(b) >= 1
                }
            }) {
                previews.push((4u8, a, b, result));
                crafted4.insert(result);
            }
        }
        // 5★：库存 4★（或由 3★ 现场合成的 4★）两两合成。
        let avail4 = |c: u32| owned(c) >= 1 || crafted4.contains(&c);
        let mut crafted5: HashSet<u32> = HashSet::new();
        for (&result, pairs) in &d.rev5 {
            if let Some(&(a, b)) = pairs.iter().find(|&&(a, b)| {
                if a == b {
                    owned(a) >= 2 || (owned(a) == 1 && crafted4.contains(&a))
                } else {
                    avail4(a) && avail4(b)
                }
            }) {
                previews.push((5u8, a, b, result));
                crafted5.insert(result);
            }
        }
        // 7★：库存 5★（或由 4★ 现场合成的 5★）两两合成。
        let avail5 = |c: u32| owned(c) >= 1 || crafted5.contains(&c);
        for (&result, pairs) in &d.rev7 {
            if let Some(&(a, b)) = pairs.iter().find(|&&(a, b)| {
                if a == b {
                    owned(a) >= 2 || (owned(a) == 1 && crafted5.contains(&a))
                } else {
                    avail5(a) && avail5(b)
                }
            }) {
                previews.push((7u8, a, b, result));
            }
        }
        previews.sort_by_key(|(star, a, _, result)| (*star, d.name(*result), d.name(*a)));
        previews
    }

    fn render_route_list(
        &mut self,
        routes: &[Route],
        offset: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let items = routes
            .iter()
            .enumerate()
            .map(|(i, route)| self.render_route(offset + i + 1, route, cx))
            .collect::<Vec<_>>();
        div()
            .id("route-list")
            .size_full()
            .overflow_y_scrollbar()
            .child(v_flex().px_4().py_2().children(items))
    }

    /// 英雄名 + 星级徽标（用于路线文本行内）。
    fn hero_chip(&self, cid: u32) -> impl IntoElement {
        h_flex()
            .gap_1()
            .items_center()
            .child(StarBadge {
                star: self.data.star(cid),
            })
            .child(
                div()
                    .text_sm()
                    .child(SharedString::from(self.data.name(cid).to_owned())),
            )
    }

    /// 单条路线行（含星级标注与收藏按钮）。
    fn render_route(
        &mut self,
        seq: usize,
        route: &Route,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let step_line = |step: &(u32, u32, u32), muted: bool| {
            let (a, b, result) = (step.0, step.1, step.2);
            let text_color = if muted {
                cx.theme().muted_foreground
            } else {
                cx.theme().foreground
            };
            h_flex()
                .gap_1p5()
                .items_center()
                .child(self.hero_chip(a))
                .child(
                    Label::new("+")
                        .text_xs()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(self.hero_chip(b))
                .child(
                    Label::new("=")
                        .text_xs()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(self.hero_chip(result))
                .text_color(text_color)
        };

        // 材料去重（保留首次出现顺序），标注份数。
        let mut mats_unique: Vec<(u32, usize)> = Vec::new(); // (cid, count)
        for m in &route.mats {
            let n = route.mats.iter().filter(|x| **x == *m).count();
            if let Some(existing) = mats_unique.iter_mut().find(|(c, _)| c == m) {
                existing.1 = n;
            } else {
                mats_unique.push((*m, n));
            }
        }
        let is_fav = self.is_favorite(route);
        let route_for_toggle = route.clone();
        let fav_button = Button::new(("route-fav", seq))
            .ghost()
            .xsmall()
            .icon(if is_fav {
                IconName::StarFill
            } else {
                IconName::Star
            })
            .tooltip(if is_fav {
                "取消收藏"
            } else {
                "收藏该路线"
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_favorite(&route_for_toggle, cx);
            }));

        let content = v_flex().flex_1().min_w_0().gap_1().child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .min_w_8()
                        .text_right()
                        .text_color(cx.theme().muted_foreground)
                        .child(SharedString::from(fmt_num(seq))),
                )
                .child(step_line(&route.steps[0], false)),
        );
        let content = content
            .children(route.steps[1..].iter().map(|step| {
                h_flex()
                    .gap_1p5()
                    .pl_16()
                    .child(
                        Icon::new(IconName::CornerDownRight)
                            .xsmall()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(step_line(step, true))
            }))
            .child(
                h_flex()
                    .gap_2()
                    .pl_16()
                    .items_start()
                    .child(
                        Label::new("材料")
                            .text_xs()
                            .pt_0p5()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_x_2()
                            .gap_y_1()
                            .items_center()
                            .children(mats_unique.iter().map(|(cid, count)| {
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .child(StarBadge {
                                        star: self.data.star(*cid),
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(SharedString::from(if *count > 1 {
                                                format!("{}×{}", self.data.name(*cid), count)
                                            } else {
                                                self.data.name(*cid).to_owned()
                                            })),
                                    )
                            })),
                    ),
            );

        h_flex()
            .id(("route", seq))
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .items_start()
            .child(content)
            .child(fav_button)
    }

    fn render_pagination(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let total_routes = if self.show_favorites {
            self.favorites.len()
        } else {
            self.visible_route_count()
        };
        let total_pages = total_routes.div_ceil(PAGE_SIZE).max(1);
        let page = self.page.clamp(1, total_pages);
        h_flex()
            .justify_center()
            .items_center()
            .px_4()
            .py_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Pagination::new("route-pagination")
                    .current_page(page)
                    .total_pages(total_pages)
                    .on_click(cx.listener(|this, page: &usize, _, cx| {
                        this.page = *page;
                        cx.notify();
                    })),
            )
    }

    fn page_slice(&self) -> Option<(Vec<Route>, usize)> {
        let res = self.results.as_ref()?;
        if res.routes.is_empty() {
            return None;
        }
        let total = self.visible_route_count();
        let total_pages = total.div_ceil(PAGE_SIZE).max(1);
        let page = self.page.clamp(1, total_pages);
        let start = (page - 1) * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(total);
        let routes = match &self.filtered {
            Some(indices) => indices[start..end]
                .iter()
                .map(|&i| res.routes[i].clone())
                .collect(),
            None => res.routes[start..end].to_vec(),
        };
        Some((routes, start))
    }

    /// 当前可见路线数（筛选后）。
    fn visible_route_count(&self) -> usize {
        match (&self.filtered, &self.results) {
            (Some(indices), Some(_)) => indices.len(),
            (_, Some(res)) => res.routes.len(),
            _ => 0,
        }
    }

    // ---------- 导出 ----------

    fn export_csv(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(res) = self.results.clone() else {
            return;
        };
        let Some(target) = self.result_target else {
            return;
        };
        let target_name = self.data.name(target).to_string();
        let data = self.data.clone();
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(format!("可合成公式_{target_name}.csv"))
            .add_filter("CSV 文件", &["csv"])
            .save_file()
        else {
            return;
        };
        let mut csv = String::from("\u{feff}序号,合成步骤,原始材料,材料份数\r\n");
        // 导出遵循当前筛选结果。
        let indices: Vec<usize> = match &self.filtered {
            Some(f) => f.as_ref().clone(),
            None => (0..res.routes.len()).collect(),
        };
        for (row, i) in indices.iter().enumerate() {
            let route = &res.routes[*i];
            let steps: Vec<String> = route
                .steps
                .iter()
                .map(|s| format!("{}+{}={}", data.name(s.0), data.name(s.1), data.name(s.2)))
                .collect();
            let mut mats: Vec<String> = Vec::new();
            for m in &route.mats {
                let name = data.name(*m).to_string();
                let n = route.mats.iter().filter(|x| **x == *m).count();
                let label = if n > 1 { format!("{name}×{n}") } else { name };
                if !mats.contains(&label) {
                    mats.push(label);
                }
            }
            csv.push_str(&format!(
                "{},{},{},{}\r\n",
                row + 1,
                steps.join("；"),
                mats.join("、"),
                route.mats.len()
            ));
        }
        match std::fs::write(&path, csv) {
            Ok(_) => {
                window.push_notification(Notification::success(path.display().to_string()), cx);
            }
            Err(e) => {
                window.push_notification(Notification::error(e.to_string()), cx);
            }
        }
    }

    fn open_account_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let account_input = self.account_input.clone();
        let app = cx.entity();
        let initial_focus = std::cell::Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                account_input.focus_handle(cx).focus(window, cx);
            }
            let app_confirm = app.clone();
            dialog
                .title("从账号导入材料")
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            Label::new("输入游戏内角色名，自动拉取账号中的可用材料（测试功能）")
                                .text_sm()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(Input::new(&account_input)),
                )
                .footer(
                    DialogFooter::new()
                        .child(Button::new("cancel-import").label("取消").on_click(
                            |_, window, cx| {
                                window.close_dialog(cx);
                            },
                        ))
                        .child(
                            Button::new("confirm-import")
                                .primary()
                                .label("导入")
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    app_confirm.update(cx, |this, cx| this.import_from_account(cx));
                                }),
                        ),
                )
        });
    }
}

impl Render for App {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status.clone();
        let update_control = self.render_update_control(cx).into_any_element();
        let title_bar = self.render_title_bar(cx).into_any_element();
        let sidebar = self.render_sidebar(cx).into_any_element();
        let results = self.render_results(cx).into_any_element();
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(title_bar)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(sidebar)
                    .child(results),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .py_1p5()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().sidebar)
                    .child(
                        Label::new(status)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(update_control),
            )
    }
}

/// 单条路线是否命中筛选条件：材料多选（任一命中）与文本搜索同时生效。
fn route_hits(
    route: &Route,
    text: &str,
    cids: &[u32],
    zh: &HashMap<u32, String>,
    en: &HashMap<u32, String>,
) -> bool {
    // 路线中出现的全部英雄（步骤参与者 + 叶子材料）。
    let mut used: Vec<u32> = route.mats.clone();
    for step in &route.steps {
        used.push(step.0);
        used.push(step.1);
        used.push(step.2);
    }
    if !cids.is_empty() && !cids.iter().any(|c| used.contains(c)) {
        return false;
    }
    if !text.is_empty()
        && !used.iter().any(|c| {
            zh.get(c).is_some_and(|n| n.contains(text))
                || en.get(c).is_some_and(|n| n.contains(text))
                || c.to_string() == text
        })
    {
        return false;
    }
    true
}

// ---------- 工具函数 ----------

fn hero_item(data: &ServerData, cid: u32, name: &str) -> HeroItem {
    HeroItem {
        cid,
        name: name.to_owned().into(),
        en: data
            .cid_to_name_en
            .get(&cid)
            .cloned()
            .unwrap_or_default()
            .into(),
        star: data.star(cid),
    }
}

fn target_items(data: &ServerData) -> SearchableVec<HeroItem> {
    SearchableVec::new(
        data.cid_to_name
            .iter()
            .filter(|(cid, _)| matches!(data.star(**cid), 4 | 5 | 7))
            .map(|(cid, name)| hero_item(data, *cid, name))
            .collect::<Vec<HeroItem>>(),
    )
}

fn mat_items(data: &ServerData) -> SearchableVec<HeroItem> {
    SearchableVec::new(
        data.cid_to_name
            .iter()
            .filter(|(cid, _)| matches!(data.star(**cid), 3..=5))
            .map(|(cid, name)| hero_item(data, *cid, name))
            .collect::<Vec<HeroItem>>(),
    )
}

/// 拆分 `名称 ×N / xN / *N / N×` 数量写法。
fn split_multiplier(item: &str) -> (&str, u32) {
    let item = item.trim();
    for sep in ['×', 'x', 'X', '*'] {
        if let Some(pos) = item.rfind(sep) {
            let (name, num) = item.split_at(pos);
            let num = &num[sep.len_utf8()..];
            if !name.trim().is_empty() && !num.is_empty() && num.chars().all(|c| c.is_ascii_digit())
            {
                return (name.trim(), num.parse().unwrap_or(1));
            }
        }
    }
    if let Some(pos) = item.find(['×', '*', 'x', 'X']) {
        let sep_char = item[pos..].chars().next().expect("separator");
        let num = &item[..pos];
        let name = &item[pos + sep_char.len_utf8()..];
        let has_non_digit = name.chars().any(|c| !c.is_ascii_digit());
        if !num.is_empty()
            && num.chars().all(|c| c.is_ascii_digit())
            && !name.trim().is_empty()
            && has_non_digit
        {
            return (name.trim(), num.parse().unwrap_or(1));
        }
    }
    (item, 1)
}

fn fmt_num(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let bytes = s.as_bytes();
    for (i, c) in s.char_indices() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 拉取账号船员并过滤出可用材料（3★-5★）。
fn fetch_account_crew(url: &str, data: &ServerData) -> Result<Vec<u32>, String> {
    let body = ureq::get(url)
        .timeout(std::time::Duration::from_secs(15))
        .set(
            "User-Agent",
            "Mozilla/5.0 (iPad; CPU OS 14_0 like Mac OS X)",
        )
        .call()
        .map_err(|e| format!("网络请求失败：{e}"))?
        .into_string()
        .map_err(|e| format!("读取响应失败：{e}"))?;
    if body.contains("errorMessage=\"User not found.\"") {
        return Err("角色名不存在，请检查后重试".into());
    }
    let marker = "CharacterDesignId=\"";
    let mut ids = Vec::new();
    let mut rest = body.as_str();
    while let Some(pos) = rest.find(marker) {
        let after = &rest[pos + marker.len()..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            rest = &after[1..];
            continue;
        }
        if let Ok(cid) = digits.parse::<u32>() {
            if matches!(data.star(cid), 3..=5) && !ids.contains(&cid) {
                ids.push(cid);
            }
        }
        rest = &after[digits.len()..];
    }
    ids.sort();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Route;

    #[test]
    fn split_multiplier_cases() {
        assert_eq!(split_multiplier("破败钢琴家×3"), ("破败钢琴家", 3));
        assert_eq!(split_multiplier("虫女士 x12"), ("虫女士", 12));
        assert_eq!(split_multiplier("3x虫女士"), ("虫女士", 3));
        assert_eq!(split_multiplier("劳拉"), ("劳拉", 1));
    }

    #[test]
    fn fmt_num_cases() {
        assert_eq!(fmt_num(90612), "90,612");
        assert_eq!(fmt_num(1), "1");
        assert_eq!(fmt_num(1234), "1,234");
    }

    #[test]
    fn route_hits_matching() {
        let zh: HashMap<u32, String> = [(14, "美杜莎".to_string()), (11, "维纳斯".to_string())]
            .into_iter()
            .collect();
        let en: HashMap<u32, String> = HashMap::new();
        let route = Route {
            steps: vec![(11, 16, 14)],
            mats: vec![11, 16],
        };
        // 文本命中结果名。
        assert!(route_hits(&route, "美杜莎", &[], &zh, &en));
        assert!(!route_hits(&route, "不存在的名字", &[], &zh, &en));
        // 材料筛选：任一命中即保留。
        assert!(route_hits(&route, "", &[11], &zh, &en));
        assert!(!route_hits(&route, "", &[99], &zh, &en));
        // 组合条件需同时满足。
        assert!(route_hits(&route, "美杜莎", &[11], &zh, &en));
        assert!(!route_hits(&route, "维纳斯", &[99], &zh, &en));
        // 按编号搜索。
        assert!(route_hits(&route, "14", &[], &zh, &en));
    }
}
