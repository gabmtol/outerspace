import en from "./i18n/en.json";

type Dict = { [key: string]: string | Dict };

const dict: Dict = en;

/** Looks up a dotted key ("power.on") and fills `{name}` placeholders. Unknown keys render as the key itself. */
export function t(key: string, vars: Record<string, string | number> = {}): string {
  const value = key.split(".").reduce<string | Dict | undefined>(
    (node, part) => (typeof node === "object" ? node[part] : undefined),
    dict,
  );
  const text = typeof value === "string" ? value : key;
  return text.replace(/\{(\w+)\}/g, (match, name: string) => (name in vars ? String(vars[name]) : match));
}

/** Fills every `data-i18n` (text) and `data-i18n-title` (tooltip + aria-label) element under `root`. */
export function applyI18n(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n!);
  });
  root.querySelectorAll<HTMLElement>("[data-i18n-title]").forEach((el) => {
    const label = t(el.dataset.i18nTitle!);
    el.title = label;
    el.setAttribute("aria-label", label);
  });
}
