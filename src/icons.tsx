// 统一尺寸的线性图标（14px，stroke 风格，与面板文字色一致）
// 三个头部操作：设置(齿轮) / 排序(上下交换) / 刷新(循环箭头)

interface IconProps {
  size?: number;
}

interface BrandIconProps extends IconProps {
  brand: string;
  label?: string;
}

function base(size: number) {
  return {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.8,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
  };
}

export function IconSettings({ size = 14 }: IconProps) {
  return (
    <svg {...base(size)}>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h.01a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h.01a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v.01a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  );
}

export function IconSort({ size = 14 }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="M11 5h10" />
      <path d="M11 9h7" />
      <path d="M11 13h4" />
      <path d="m3 17 3 3 3-3" />
      <path d="M6 18V4" />
    </svg>
  );
}

export function IconCheck({ size = 14 }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="M20 6 9 17l-5-5" />
    </svg>
  );
}

export function IconArrowLeft({ size = 16 }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="m15 18-6-6 6-6" />
    </svg>
  );
}

export function IconChevronDown({ size = 15 }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="m7 10 5 5 5-5" />
    </svg>
  );
}

export function IconPlan({ size = 18 }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="m12 3 7 4v10l-7 4-7-4V7l7-4Z" />
      <path d="m8.5 11.5 2.2 2.2 4.8-5" />
    </svg>
  );
}

export function IconApi({ size = 18 }: IconProps) {
  return (
    <svg {...base(size)}>
      <circle cx="6" cy="12" r="2.2" />
      <circle cx="17.5" cy="6" r="2.2" />
      <circle cx="17.5" cy="18" r="2.2" />
      <path d="m8 11 7.5-4M8 13l7.5 4" />
    </svg>
  );
}

export function IconRefresh({ size = 14 }: IconProps) {
  return (
    <svg {...base(size)}>
      <path d="M21 12a9 9 0 1 1-2.64-6.36" />
      <path d="M21 3v6h-6" />
    </svg>
  );
}

/** TokenMeter 的紧凑表盘标记，用于面板标题，不复用深色 App 图标底板。 */
export function IconGauge({ size = 20 }: IconProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="M4.4 16.8a8.2 8.2 0 1 1 15.2 0" stroke="#21a878" strokeWidth="3" strokeLinecap="round" />
      <path d="M5.05 8.85A8.2 8.2 0 0 1 12 3.8" stroke="#75d59a" strokeWidth="3" strokeLinecap="round" />
      <path d="m12 14.1 4.2-4.1" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
      <circle cx="12" cy="14.1" r="2" fill="currentColor" />
    </svg>
  );
}

/**
 * 卡片品牌标记。它只承担快速识别，不再额外重复显示“平台”字段。
 * 图形保持单色，颜色由卡片的品牌 token 控制。
 */
export function BrandIcon({ brand, label, size = 22 }: BrandIconProps) {
  if (brand === "anthropic") {
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M4.2 19 10.7 4.8h2.6L19.8 19M7.2 15h9.6M9.2 19l5.6-14.2" stroke="currentColor" strokeWidth="2.15" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    );
  }

  if (brand === "openrouter") {
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M2.5 9.1c2.8 0 4.8-2.5 7.2-4 2.7-1.7 5-1.4 8.1-1.4" stroke="currentColor" strokeWidth="2.25" strokeLinecap="round" />
        <path d="m17 1.5 4 2.2-4 2.2V1.5Z" fill="currentColor" />
        <path d="M2.5 14.9c2.8 0 4.8 2.5 7.2 4 2.7 1.7 5 1.4 8.1 1.4" stroke="currentColor" strokeWidth="2.25" strokeLinecap="round" />
        <path d="m17 18.1 4 2.2-4 2.2v-4.4Z" fill="currentColor" />
      </svg>
    );
  }

  if (brand === "kimi") {
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M5.5 4v16M6 12h4.4M10.4 12 17 4M10.4 12l7.1 8M18.5 4v4" stroke="currentColor" strokeWidth="2.7" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    );
  }

  if (brand === "moonshot") {
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M18.2 16.9A8.1 8.1 0 0 1 8 5.3a8.1 8.1 0 1 0 10.2 11.6Z" fill="currentColor" />
        <circle cx="17.7" cy="6.1" r="1.7" fill="currentColor" opacity=".55" />
      </svg>
    );
  }

  if (brand === "deepseek") {
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M3.2 13.4c2.2 3.9 6.1 5.8 10.2 4.6 3.1-.9 5.4-3.2 6.8-6.1-1.7.8-3.3.7-4.7-.3-1.8-1.4-3.7-2.2-6.1-1.7-2.4.5-4.4 1.8-6.2 3.5Z" fill="currentColor" />
        <path d="M13.9 9.8c.7-2.4 2.3-4.1 4.8-4.8-.1 2.2-.9 4-2.5 5.4" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" />
        <circle cx="7.6" cy="13.2" r="1" fill="white" />
      </svg>
    );
  }

  if (brand === "glm") {
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M4.2 7.1 12 2.9l7.8 4.2v9.8L12 21.1l-7.8-4.2V7.1Z" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round" />
        <path d="M7.4 8.1h9.2l-9.2 7.8h9.2M12 2.9v5.2M12 15.9v5.2" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
      </svg>
    );
  }

  if (brand === "minimax") {
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M3.5 17.8V7.1l4.4 6.5L12 7.1l4.1 6.5 4.4-6.5v10.7" stroke="currentColor" strokeWidth="2.25" strokeLinecap="round" strokeLinejoin="round" />
        <path d="M4.1 19.9h15.8" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" opacity=".58" />
      </svg>
    );
  }

  if (brand === "tencent") {
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M6 7.2h12M12 7.2V20M4.2 4h15.6" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" />
      </svg>
    );
  }

  if (brand === "openai") {
    // OpenAI / Codex：用六向联结图形表达模型网络，避免加入文字缩写。
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <path d="M12 3.2a4.3 4.3 0 0 1 4.1 3 4.3 4.3 0 0 1 2.5 6.3 4.3 4.3 0 0 1-1.6 6.5 4.3 4.3 0 0 1-6.6.8 4.3 4.3 0 0 1-5.8-3.4 4.3 4.3 0 0 1-.8-6.6A4.3 4.3 0 0 1 8 4.5 4.3 4.3 0 0 1 12 3.2Z" stroke="currentColor" strokeWidth="1.75" strokeLinejoin="round" />
        <path d="m8 4.5 7.9 13.2M3.8 9.8 17 19M4.6 16.4 16.1 6.2M10.4 19.8l8.2-7.3M18.6 12.5 8 4.5" stroke="currentColor" strokeWidth="1.35" strokeLinecap="round" opacity=".9" />
      </svg>
    );
  }

  // 未内置图标的新供应商自动回退到名称首字，不阻塞标准卡片生成。
  const initial = label?.trim().charAt(0).toUpperCase() || "•";
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <text x="12" y="16" textAnchor="middle" fill="currentColor" fontSize="13" fontWeight="700">
        {initial}
      </text>
    </svg>
  );
}
