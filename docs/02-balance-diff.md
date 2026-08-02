# 余额差分算法

> 解决 Moonshot / DeepSeek 无官方用量接口的问题。
> 目标：仅凭「余额」这一个可靠信号，推算出日 / 周 / 月消耗金额。

---

## 问题

Moonshot 和 DeepSeek 都只暴露当前余额，不提供任何用量聚合接口。用户要看的日/周/月消耗，官方给不了。

但余额本身是一条时间序列。对一个**只减不增**的账户，相邻两次采样的差值就是这段时间的消耗。真正要处理的是「增」的那些时刻——充值、赠送额度发放、代金券过期。

---

## 核心公式

```
消耗(t1, t2) = Σ max(0, balance[i] - balance[i+1])   for i in [t1, t2)
```

对每个相邻采样对取**下降量**，上升一律记 0，然后累加。这样充值造成的跳升被自然吸收，不会污染消耗统计。

---

## 数据模型

```sql
CREATE TABLE balance_snapshot (
    id           INTEGER PRIMARY KEY,
    provider_id  TEXT    NOT NULL,
    captured_at  INTEGER NOT NULL,      -- unix 秒
    total        REAL    NOT NULL,      -- 总余额
    granted      REAL,                  -- 赠送额度（DeepSeek 有，Moonshot 记 voucher）
    topped_up    REAL,                  -- 充值余额（DeepSeek 有，Moonshot 记 cash）
    currency     TEXT    NOT NULL,
    source       TEXT    NOT NULL       -- 'poll' | 'manual' | 'wake'
);

CREATE INDEX idx_snap ON balance_snapshot(provider_id, captured_at DESC);

CREATE TABLE balance_event (
    id           INTEGER PRIMARY KEY,
    provider_id  TEXT    NOT NULL,
    occurred_at  INTEGER NOT NULL,
    kind         TEXT    NOT NULL,      -- 'topup' | 'grant' | 'expire' | 'anomaly'
    delta        REAL    NOT NULL,
    note         TEXT
);
```

分字段存 `granted` / `topped_up` 是关键——DeepSeek 分开返回这两项，能精确区分「赠送额度过期」和「真实消耗」，避免把过期误算成花钱。

---

## 分类规则

设 `d = curr.total - prev.total`，`ε = 0.001`（浮点容差）。

| 条件 | 判定 | 处理 |
|------|------|------|
| `d < -ε` | 正常消耗 | 累加 `-d` 到消耗 |
| `d > ε` 且 `topped_up` 上升 | 充值 | 记 `topup` 事件，消耗记 0 |
| `d > ε` 且 `granted` 上升 | 赠送发放 | 记 `grant` 事件，消耗记 0 |
| `d < -ε` 且 `granted` 下降但 `topped_up` 不变，且降幅 = granted 全额 | 代金券过期 | 记 `expire` 事件，**不计入消耗** |
| `\|d\| <= ε` | 无变化 | 跳过 |

第四条是最容易出错的地方。代金券整额过期时余额会突然掉一大块，如果不识别就会在图表上出现一根假的消费尖峰。判据是「granted 归零或整额下降 + topped_up 纹丝不动」。

---

## 实现

```rust
pub struct DiffResult {
    pub consumed: f64,
    pub events: Vec<BalanceEvent>,
    pub coverage: f64,   // 采样覆盖率，0.0 ~ 1.0
}

pub fn diff_range(snaps: &[Snapshot], from: i64, to: i64) -> DiffResult {
    let mut consumed = 0.0;
    let mut events = Vec::new();

    let window: Vec<&Snapshot> = snaps.iter()
        .filter(|s| s.captured_at >= from && s.captured_at <= to)
        .collect();

    for pair in window.windows(2) {
        let (prev, curr) = (pair[0], pair[1]);
        let d = curr.total - prev.total;

        if d.abs() <= EPSILON {
            continue;
        }

        if d > EPSILON {
            let kind = if curr.topped_up.zip(prev.topped_up)
                          .map_or(false, |(c, p)| c - p > EPSILON) {
                EventKind::Topup
            } else {
                EventKind::Grant
            };
            events.push(BalanceEvent::new(curr.captured_at, kind, d));
            continue;
        }

        // d < 0：可能是消耗，也可能是代金券过期
        if is_voucher_expiry(prev, curr) {
            events.push(BalanceEvent::new(curr.captured_at, EventKind::Expire, d));
            continue;
        }

        consumed += -d;
    }

    DiffResult { consumed, events, coverage: coverage_of(&window, from, to) }
}

fn is_voucher_expiry(prev: &Snapshot, curr: &Snapshot) -> bool {
    match (prev.granted, curr.granted, prev.topped_up, curr.topped_up) {
        (Some(pg), Some(cg), Some(pt), Some(ct)) => {
            let granted_dropped = pg - cg > EPSILON;
            let topped_stable   = (pt - ct).abs() <= EPSILON;
            let drop_is_full    = cg.abs() <= EPSILON;
            granted_dropped && topped_stable && drop_is_full
        }
        _ => false,
    }
}
```

---

## 采样策略

固定间隔轮询会漏掉两类场景：机器休眠期间的消耗、以及两次采样之间的充值+消耗组合。

采用**自适应间隔 + 唤醒补采**：

```
活跃期（App 面板打开）        60 秒
常态（后台驻留）             5 分钟
空闲（连续 6 次余额无变化）   15 分钟
系统唤醒 / 网络恢复          立即补采一次，source='wake'
手动刷新                     立即，source='manual'
```

空闲降频能显著减少无效请求。DeepSeek 和 Moonshot 的 balance 接口都没有明确限频文档，保守处理避免触发风控。

---

## 覆盖率与可信度

差分结果的可信度取决于采样密度。定义：

```
coverage = 实际采样点数 / 理论应有采样点数
```

对应 UI 呈现：

| 覆盖率 | 标注 | 说明 |
|--------|------|------|
| ≥ 0.95 | 无标注 | 数据可信 |
| 0.7 – 0.95 | 「约」前缀 | 有采样缺口 |
| < 0.7 | 「数据不完整」灰色提示 | 机器长时间关机 |

**绝不把推算值伪装成精确值。** 差分出来的是「App 运行期间观测到的消耗」，机器关机时段的消耗永远拿不到。第一次添加供应商时也没有历史基线，头 24 小时应显示「统计中」而非 0。

---

## 已知局限

1. **拿不到 token 数** — 只有金额。要 token 数必须走逆向路径。
2. **关机盲区** — 无法覆盖。可用「余额总降幅 vs 差分累计」的偏差量反推盲区消耗，作为补充展示。
3. **多端共用同一 Key** — 差分统计的是账户级消耗，不区分是哪台机器/哪个工具花的。这其实符合预期。
4. **精度** — 余额接口返回的小数位有限（Moonshot 5 位、DeepSeek 2 位字符串）。单次极小额调用可能被精度吞掉，长期累计误差可忽略。

---

## 与逆向路径的混合

按你选的「混合 + 自动降级」：

```
优先级 1  控制台内部接口   → token 数 + 金额 + 模型拆分 + 缓存命中率
优先级 2  余额差分         → 仅金额
```

每次刷新先试优先级 1，连续失败 3 次则标记该路径为 `degraded`，切到优先级 2，并在 UI 该卡片右上角显示一个小圆点：

- 实心 — 精确数据（内部接口）
- 空心 — 推算数据（余额差分）

鼠标悬停显示数据来源与最后成功时间。降级后每 6 小时静默重试一次优先级 1，恢复则自动切回。
