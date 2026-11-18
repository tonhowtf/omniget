/**
 * Site-helper cards get their description from the remote catalog
 * (`catalog.json`), which only ships English text. Keep the Russian
 * wording here, keyed by connector id, so the market reads naturally
 * without changing the catalog contract.
 *
 * Unknown ids fall back to the catalog text, so a new helper still
 * renders; it just stays in English until it is added below.
 */
export const RU_CONNECTOR_DESCRIPTIONS: Record<string, string> = {
  "100tasks":
    "Что открывает ваш аккаунт на 100tasks.com: 100 заданий и обзоры 100 Tasks AI в Markdown, плюс доступные вам уроки курсов access.100tasks.com (тексты, PDF, видео).",
  branas:
    "Курсы, на которые вы записаны на branas.io и в других школах LaunchLab: видео (Panda Video, YouTube, Vimeo и другие), PDF, аудио, файлы и текстовые уроки — по модулям.",
  entregadigital:
    "Закрытые кабинеты Entrega Digital (*.entregadigital.app.br), например Reservatório de Dopamina: видео (Panda Video, YouTube), расшифровки уроков, PDF, таблицы, уроки в HTML и ссылки — по продукту и модулю. Один продукт или весь каталог.",
  fisicacomrenatobrito:
    "Курсы, на которые вы записаны на app.fisicacomrenatobrito.com.br: видео (Videoteca EAD HLS, YouTube, Vimeo), PDF модулей и описания — по разделам и модулям.",
  ggcheckout:
    "Закрытые разделы, к которым у вас есть доступ на ggcheckout.app: уроки (YouTube, Vimeo, HLS, прямые видео), тексты, файлы и вложения — по модулям и разделам.",
  hotmart:
    "Курсы Hotmart Club, которые вы купили: видео, аудио, вложения и тексты уроков — по модулям. Закрытые уроки помечаются. Материалы с DRM показываются и пропускаются.",
  "tle-eliminators":
    "Ваш курс TLE Eliminators (панель): все модули, наборы лекций и задач с английскими субтитрами, слайдами, кодом, разборами и списками задач. Видеолекции используют DRM TPStreams — они показываются и пропускаются.",
  udemy:
    "Курсы, на которые вы записаны: видео, субтитры, статьи, файлы, вложения, тесты и практические задания. Лекции с DRM показываются и пропускаются.",
};

/** Catalog description for `id`, translated when the app runs in Russian. */
export function localizedConnectorDescription(
  id: string,
  fallback: string | undefined,
  activeLocale: string,
): string | undefined {
  if (activeLocale === "ru") return RU_CONNECTOR_DESCRIPTIONS[id] ?? fallback;
  return fallback;
}
