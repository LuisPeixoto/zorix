// Dates, times, numbers, money and lists for the interface (ADR-0040), through `Intl` in the active locale. Nothing else formats a value.

/** Formatters for one locale. */
export interface Formats {
  /** A whole number or a decimal, grouped as the locale does (`12,345`). */
  number(value: number, options?: Intl.NumberFormatOptions): string;
  /** A share from 0 to 1 as a percentage (`94 %` in some locales, `94%` in English). */
  percent(value: number, digits?: number): string;
  /** An amount in US dollars, given in millionths of a dollar. Under a cent it shows four decimals so a small cost is not zero. */
  usd(micros: number): string;
  /** An amount in US dollars, given in millionths of a dollar, always with cents (`$9.80`). */
  usd2(micros: number): string;
  /** An amount in US dollars, given in millionths of a dollar, with four decimals (`$0.0042`): the price of a single call. */
  usd4(micros: number): string;
  /** An amount in US dollars in whole dollars when it is one (`$25`), else with cents (`$25.50`). */
  dollars(value: number): string;
  /** A calendar day: `Dec 12`. */
  day(at: number | Date, options?: Intl.DateTimeFormatOptions): string;
  /** A time of day, 24-hour: `14:05`, or `14:05:09` with seconds. */
  clock(at: number | Date, seconds?: boolean): string;
  /** A date and a time, for a record's own stamp. */
  stamp(at: number | Date): string;
  /** A day of the week, short (`Mon`) or long. */
  weekday(at: number | Date, style?: "short" | "long"): string;
  /** A list in words: `a, b and c`. */
  list(items: string[], type?: "conjunction" | "disjunction"): string;
  /** A size in bytes: `2 KB`. */
  bytes(value: number): string;
}

/** The locale whose list style we use: English written without the serial comma, as the interface always has. */
const listLocale = (locale: string) => (locale === "en" ? "en-GB" : locale);

const asDate = (at: number | Date) => (at instanceof Date ? at : new Date(at));

export function formatsFor(locale: string): Formats {
  const intl = locale === "en-XA" ? "en" : locale;
  return {
    number: (value, options) => new Intl.NumberFormat(intl, options).format(value),
    percent: (value, digits = 0) =>
      new Intl.NumberFormat(intl, { style: "percent", maximumFractionDigits: digits, minimumFractionDigits: digits }).format(value),
    usd: (micros) => {
      const dollars = micros / 1_000_000;
      const digits = Math.abs(dollars) > 0 && Math.abs(dollars) < 0.01 ? 4 : 2;
      return new Intl.NumberFormat(intl, { style: "currency", currency: "USD", minimumFractionDigits: digits, maximumFractionDigits: digits }).format(dollars);
    },
    usd2: (micros) =>
      new Intl.NumberFormat(intl, { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 2 }).format(micros / 1_000_000),
    usd4: (micros) =>
      new Intl.NumberFormat(intl, { style: "currency", currency: "USD", minimumFractionDigits: 4, maximumFractionDigits: 4 }).format(micros / 1_000_000),
    dollars: (value) =>
      new Intl.NumberFormat(intl, {
        style: "currency",
        currency: "USD",
        minimumFractionDigits: Number.isInteger(value) ? 0 : 2,
        maximumFractionDigits: 2,
      }).format(value),
    day: (at, options) => new Intl.DateTimeFormat(intl, options ?? { month: "short", day: "numeric" }).format(asDate(at)),
    clock: (at, seconds = false) =>
      new Intl.DateTimeFormat(intl, { hour: "2-digit", minute: "2-digit", ...(seconds ? { second: "2-digit" } : {}), hourCycle: "h23" }).format(asDate(at)),
    stamp: (at) => new Intl.DateTimeFormat(intl, { dateStyle: "medium", timeStyle: "short", hourCycle: "h23" }).format(asDate(at)),
    weekday: (at, style = "short") => new Intl.DateTimeFormat(intl, { weekday: style }).format(asDate(at)),
    list: (items, type = "conjunction") => new Intl.ListFormat(listLocale(intl), { style: "long", type }).format(items),
    bytes: (value) => {
      if (value < 1024) return `${new Intl.NumberFormat(intl).format(value)} B`;
      const one = new Intl.NumberFormat(intl, { minimumFractionDigits: 1, maximumFractionDigits: 1 });
      if (value < 1024 * 1024) return `${one.format(value / 1024)} KB`;
      return `${one.format(value / (1024 * 1024))} MB`;
    },
  };
}
