import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import { en, type Resources } from "./locales/en/index";
import { pt } from "./locales/pt/index";

/** Idiomas oferecidos no seletor das configurações. */
export const SUPPORTED_LANGUAGES = [
  { code: "en", label: "English", locale: "en-US" },
  { code: "pt", label: "Português (Brasil)", locale: "pt-BR" },
] as const;

export type LanguageCode = (typeof SUPPORTED_LANGUAGES)[number]["code"];

/** Idioma padrão quando não há preferência salva. */
const DEFAULT_LANGUAGE: LanguageCode = "en";

const STORAGE_KEY = "slot2sync.language";

function isSupported(code: string): code is LanguageCode {
  return SUPPORTED_LANGUAGES.some((l) => l.code === code);
}

/** Idioma salvo pelo usuário, ou o padrão (inglês). */
export function storedLanguage(): LanguageCode {
  const saved = localStorage.getItem(STORAGE_KEY);
  return saved && isSupported(saved) ? saved : DEFAULT_LANGUAGE;
}

/** Troca o idioma ativo e persiste a escolha. */
export async function changeLanguage(code: LanguageCode): Promise<void> {
  localStorage.setItem(STORAGE_KEY, code);
  await i18n.changeLanguage(code);
}

/** Locale BCP-47 do idioma ativo — para `toLocaleString`/`Intl`. */
export function currentLocale(): string {
  const lang = SUPPORTED_LANGUAGES.find((l) => l.code === i18n.language);
  return lang?.locale ?? "en-US";
}

// Mantém `<html lang>` em sincronia com o idioma ativo (acessibilidade/SEO).
function syncHtmlLang(code: string) {
  const lang = SUPPORTED_LANGUAGES.find((l) => l.code === code);
  document.documentElement.lang = lang?.locale ?? code;
}
i18n.on("languageChanged", syncHtmlLang);

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    pt: { translation: pt },
  },
  lng: storedLanguage(),
  fallbackLng: DEFAULT_LANGUAGE,
  interpolation: { escapeValue: false }, // React já escapa
});

syncHtmlLang(i18n.language);

export default i18n;

// Tipagem estrita de `t()` — chaves e namespaces conferidos pelo TypeScript.
declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "translation";
    resources: { translation: Resources };
  }
}
