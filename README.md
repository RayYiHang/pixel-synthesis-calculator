# 像素星舰合成计算器

《像素星舰》（Pixel Starships）英雄合成路线计算器 —— 原命令行版（合成计算器Ⅱ，by 六芒龙）的纯 Rust GUI 重制版。

输入目标英雄与已有材料，枚举全部可合成路线：

- 3★ + 3★ → 4★
- 4★ + 4★ → 5★
- 5★ + 5★ → 7★

## 功能

- 目标英雄模糊搜索（中文名 / 英文名 / 编号 / 星级）
- 材料录入：多选搜索、粘贴批量导入（支持 `×N` 数量）、逐项调整份数
- 从账号导入：输入游戏内角色名自动拉取账号可用材料（测试功能）
- 国际服 / 国服数据切换（国服数据已停止维护）
- 深浅色主题、分页结果、导出 CSV

## 下载

前往 [Releases](../../releases) 页面下载：

| 平台 | 产物 |
| --- | --- |
| Windows x64 | `PixelSynthesisCalculator-windows-x64.zip`（解压即用） |
| macOS（Intel + Apple Silicon） | `PixelSynthesisCalculator-macos-universal.dmg` |
| Linux amd64 (Debian/Ubuntu) | `pixel-synthesis-calculator_*_amd64.deb` |

> macOS 产物未做代码签名，首次打开若被 Gatekeeper 拦截：右键 App → 打开，或执行 `xattr -cr /Applications/PixelSynthesisCalculator.app`。

## 从源码构建

依赖 Rust 1.88+（`rustup` / Homebrew 安装均可）：

```sh
cargo run --release
```

Linux 需先安装构建依赖：

```sh
sudo apt-get install -y pkg-config libfontconfig1-dev libfreetype-dev \
  libxkbcommon-dev libxcb1-dev libwayland-dev libvulkan-dev
```

## 打包

- macOS：`scripts/package-macos.sh`（需先 `lipo` 合并双架构二进制）
- Debian：`scripts/package-deb.sh`
- 三平台自动打包发布：推送 `v*` 标签，由 GitHub Actions（[release.yml](.github/workflows/release.yml)）完成

## 数据说明

`assets/hcdata/` 内的合成公式数据整理自原版工具的数据文件，仅供学习交流；结果仅供参考，请自行核对。

## 致谢

- 原版工具：合成计算器Ⅱ by 六芒龙
- GUI：[gpui-kit](https://github.com/longbridge/gpui-kit)
