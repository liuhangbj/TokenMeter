import type { AppAppearance, AppTheme } from "./types";

const DARK_QUERY = "(prefers-color-scheme: dark)";

export function resolveAppearance(appearance: AppAppearance): "light" | "dark" {
  if (appearance === "light" || appearance === "dark") return appearance;
  return window.matchMedia(DARK_QUERY).matches ? "dark" : "light";
}

export function applyTheme(theme: AppTheme, appearance: AppAppearance): void {
  const root = document.documentElement;
  root.dataset.theme = theme;
  root.dataset.appearance = resolveAppearance(appearance);
  root.dataset.appearancePreference = appearance;
  localStorage.setItem("tm_theme", theme);
  localStorage.setItem("tm_appearance", appearance);
}

export function readStoredTheme(): AppTheme {
  const value = localStorage.getItem("tm_theme");
  return value === "parchment" || value === "cyberpunk" ? value : "classic";
}

export function readStoredAppearance(): AppAppearance {
  const value = localStorage.getItem("tm_appearance");
  return value === "light" || value === "dark" ? value : "system";
}

export function subscribeSystemAppearance(callback: () => void): () => void {
  const query = window.matchMedia(DARK_QUERY);
  query.addEventListener("change", callback);
  return () => query.removeEventListener("change", callback);
}
