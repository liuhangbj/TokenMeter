#!/usr/bin/env bash
# ============================================================
# TokenMeter 本机测试构建 + 部署脚本（macOS）
#
# 用法:
#   scripts/dev-deploy.sh                # debug 构建 → 安装到 ~/Applications → 启动
#   scripts/dev-deploy.sh --release      # release 构建
#   scripts/dev-deploy.sh --no-install   # 只构建+启动，不安装
#   scripts/dev-deploy.sh --install-system # 安装到 /Applications（需要管理员密码）
#   scripts/dev-deploy.sh --no-run       # 只构建，不启动
#   scripts/dev-deploy.sh --isolate      # 使用独立数据目录（不影响正式版凭证/设置）
#
# 说明:
#   - 产物: src-tauri/target/<profile>/tokenmeter（二进制）
#           src-tauri/target/<profile>/bundle/macos/TokenMeter.app（.app）
#   - 启动日志: /tmp/tokenmeter-dev.log
#   - 退出 dev 实例: kill "$(cat /tmp/tokenmeter-dev.pid)"
#   - 如果正式版 TokenMeter 正在运行，dev 实例会因单实例锁直接退出；
#     测试前请先退出正式版（或只用 --no-run）。
# ============================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="debug"
RUN=1
INSTALL=1
INSTALL_SYSTEM=0
ISOLATE=0
PID_FILE="/tmp/tokenmeter-dev.pid"
LOG_FILE="/tmp/tokenmeter-dev.log"

say()  { printf '\033[1;34m%s\033[0m\n' "$*"; }
warn() { printf '\033[1;33m%s\033[0m\n' "$*" >&2; }
die()  { printf '\033[1;31m%s\033[0m\n' "$*" >&2; exit 1; }

usage() {
  sed -n '2,16p' "${BASH_SOURCE[0]}"
}

for arg in "$@"; do
  case "$arg" in
    --release) PROFILE="release" ;;
    --install) INSTALL=1 ;;
    --no-install) INSTALL=0 ;;
    --install-system) INSTALL=1; INSTALL_SYSTEM=1 ;;
    --no-run)  RUN=0 ;;
    --isolate) ISOLATE=1 ;;
    -h|--help) usage; exit 0 ;;
    *) die "未知参数: $arg（--help 查看用法）" ;;
  esac
done

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "缺少依赖: $1（请先安装）"
}

need_cmd node
need_cmd npm
need_cmd cargo

cd "$ROOT"

say "==> 构建前端 + Rust ($PROFILE)"
TAURI_ARGS=(--bundles app)
if [ "$PROFILE" = "debug" ]; then
  TAURI_ARGS+=(--debug)
fi
# 显式启用 custom-protocol：独立二进制必须内嵌前端资源，
# 否则面板会去连 devUrl（localhost:1420）导致"无法访问此页面"。
TAURI_ARGS+=(--features custom-protocol)
# 本机没有签名私钥时跳过 updater 产物（app.tar.gz + .sig），
# 否则 tauri CLI 会因为"有公钥无私钥"而中断打包。
if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  TAURI_ARGS+=(--config '{"bundle":{"createUpdaterArtifacts":false}}')
  say "==> 未设置 TAURI_SIGNING_PRIVATE_KEY，跳过 updater 签名产物"
fi
npx tauri build "${TAURI_ARGS[@]}"

APP="$ROOT/src-tauri/target/$PROFILE/bundle/macos/TokenMeter.app"
BIN="$ROOT/src-tauri/target/$PROFILE/tokenmeter"
[ -d "$APP" ] || die "未找到 .app 产物: $APP"
[ -x "$BIN" ] || die "未找到二进制产物: $BIN"

# ---- 安装到 Applications（默认 ~/Applications；--install-system 到 /Applications）----
if [ "$INSTALL" = "1" ]; then
  if [ "$INSTALL_SYSTEM" = "1" ]; then
    DEST="/Applications/TokenMeter Dev.app"
    say "==> 安装到 $DEST（需要管理员密码）"
    # 先停掉已安装的 dev 实例（按完整路径匹配，不会误杀正式版）
    pkill -f "$DEST/Contents/MacOS/tokenmeter" 2>/dev/null || true
    if [ -d "$DEST" ]; then
      # 旧包移入废纸篓（可恢复），避免 rm -rf
      sudo mv "$DEST" "$HOME/.Trash/TokenMeter Dev.app.$(date +%Y%m%d-%H%M%S)" 2>/dev/null || sudo rm -rf "$DEST"
    fi
    sudo ditto "$APP" "$DEST"
  else
    DEST="$HOME/Applications/TokenMeter Dev.app"
    say "==> 安装到 $DEST"
    mkdir -p "$HOME/Applications"
    # 先停掉已安装的 dev 实例（按完整路径匹配，不会误杀正式版）
    pkill -f "$DEST/Contents/MacOS/tokenmeter" 2>/dev/null || true
    if [ -d "$DEST" ]; then
      # 旧包移入废纸篓（可恢复），避免 rm -rf
      mv "$DEST" "$HOME/.Trash/TokenMeter Dev.app.$(date +%Y%m%d-%H%M%S)" 2>/dev/null || rm -rf "$DEST"
    fi
    ditto "$APP" "$DEST"
  fi
  BIN="$DEST/Contents/MacOS/tokenmeter"
fi

# ---- 启动（可选）----
if [ "$RUN" = "1" ]; then
  BUNDLE_TO_LAUNCH="$APP"
  if [ "$INSTALL" = "1" ]; then
    BUNDLE_TO_LAUNCH="$DEST"
  fi

  say "==> 启动已部署的 App: $BUNDLE_TO_LAUNCH"
  # 停掉上一次 dev 实例（PID 文件方式，不影响正式版）
  if [ -f "$PID_FILE" ]; then
    OLD_PID="$(cat "$PID_FILE" 2>/dev/null || true)"
    if [ -n "$OLD_PID" ] && kill -0 "$OLD_PID" 2>/dev/null; then
      say "==> 停止旧 dev 实例 (PID $OLD_PID)"
      kill "$OLD_PID" 2>/dev/null || true
      sleep 1
    fi
  fi
  # 按部署路径兜底清理（防止 PID 文件丢失后残留 dev 实例）
  pkill -f "$BUNDLE_TO_LAUNCH/Contents/MacOS/tokenmeter" 2>/dev/null || true

  # 单实例锁提示：正式版在跑时 dev 实例会直接退出
  if pgrep -x tokenmeter >/dev/null 2>&1; then
    warn "检测到已有 tokenmeter 进程（可能是正式版）：dev 实例可能因单实例锁退出，请先退出正式版"
  fi

  # 用 open 启动（LaunchServices 托管进程，不随脚本/终端退出被杀），
  # 通过 --env 传日志与隔离数据目录。
  OPEN_ARGS=(-n --env "TOKENMETER_LOG_FILE=$LOG_FILE" --stdout "$LOG_FILE" --stderr "$LOG_FILE")
  if [ "$ISOLATE" = "1" ]; then
    OPEN_ARGS+=(--env "TOKENMETER_DATA_DIR=$DATA_DIR")
  fi
  open "${OPEN_ARGS[@]}" "$BUNDLE_TO_LAUNCH"

  # open 不返回 PID，轮询获取最新实例
  NEW_PID=""
  for _ in $(seq 1 10); do
    NEW_PID="$(pgrep -n -f "$BUNDLE_TO_LAUNCH/Contents/MacOS/tokenmeter" 2>/dev/null || true)"
    [ -n "$NEW_PID" ] && break
    sleep 1
  done
  if [ -z "$NEW_PID" ]; then
    warn "未能检测到 App 进程（可能因单实例锁退出或启动即崩溃）。"
    warn "日志: $LOG_FILE"
    tail -n 20 "$LOG_FILE" 2>/dev/null || true
    exit 1
  fi
  echo "$NEW_PID" > "$PID_FILE"
  say "==> App 已启动 (PID $NEW_PID)，托盘图标应已出现"
fi

say "==> 完成"
echo "  profile : $PROFILE"
echo "  app     : $APP"
echo "  binary  : $BIN"
if [ "$RUN" = "1" ]; then
  echo "  pid     : $(cat "$PID_FILE")"
  echo "  log     : $LOG_FILE"
  echo "  停止    : kill \$(cat $PID_FILE)"
fi
if [ "$ISOLATE" = "1" ]; then
  echo "  数据目录: $HOME/Library/Application Support/TokenMeter-Dev"
fi
