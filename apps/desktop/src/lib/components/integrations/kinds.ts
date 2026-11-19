// What each kind of connection asks for. The interface only collects it; the rules live in the hub (Rust).
import type { ConnectionKind } from "../../api.ts";

export type FieldSpec = { key: string; label: string; placeholder?: string; hint?: string; required?: boolean; number?: boolean; file?: boolean };
export type KindSpec = {
  title: string;
  summary: string;
  logo: string;
  fields: FieldSpec[];
  secret?: { label: string; hint: string; required: boolean };
  grant: string;
};

export const KINDS: Record<ConnectionKind, KindSpec> = {
  github: {
    title: "GitHub",
    summary: "Issues, pull requests and files of repositories you choose.",
    logo: "github",
    fields: [{ key: "base_url", label: "API address", placeholder: "https://api.github.com", hint: "Only for GitHub Enterprise." }],
    secret: { label: "Personal access token", hint: "Leave empty to read public repositories. Writing and private repositories need a token.", required: false },
    grant: "--allow-repo owner/repo[:write]",
  },
  gitlab: {
    title: "GitLab",
    summary: "Issues, merge requests and files, on gitlab.com or your own server.",
    logo: "gitlab",
    fields: [{ key: "base_url", label: "Address", placeholder: "https://gitlab.com", hint: "Only for your own GitLab." }],
    secret: { label: "Access token", hint: "Leave empty to read public projects.", required: false },
    grant: "--allow-repo gitlab:group/project[:write]",
  },
  jira: {
    title: "Jira",
    summary: "Search, read, create, comment and move issues in projects you choose.",
    logo: "jira",
    fields: [
      { key: "base_url", label: "Site address", placeholder: "https://acme.atlassian.net", required: true },
      { key: "email", label: "Account email", required: true },
    ],
    secret: { label: "API token", hint: "Made at id.atlassian.com under Security.", required: true },
    grant: "--allow-jira KEY[:write]",
  },
  slack: {
    title: "Slack",
    summary: "Read channels and send messages, always asking you first.",
    logo: "slack",
    fields: [],
    secret: { label: "Bot token", hint: "Starts with xoxb-. Made in your Slack app under OAuth & Permissions.", required: true },
    grant: "--allow-chat slack:#channel[:write]",
  },
  discord: {
    title: "Discord",
    summary: "Read channels and send messages as a bot, always asking you first.",
    logo: "discord",
    fields: [],
    secret: { label: "Bot token", hint: "Made in the Discord developer portal under Bot.", required: true },
    grant: "--allow-chat discord:<channel id>[:write]",
  },
  postgres: {
    title: "Postgres",
    summary: "Query a database in read-only transactions. Writes always ask.",
    logo: "postgres",
    fields: [
      { key: "host", label: "Host", required: true },
      { key: "port", label: "Port", placeholder: "5432", number: true },
      { key: "database", label: "Database", required: true },
      { key: "user", label: "User", required: true },
    ],
    secret: { label: "Password", hint: "Leave empty for trust or certificate login.", required: false },
    grant: "--allow-db name[:write]",
  },
  sqlite: {
    title: "SQLite",
    summary: "Read a database file on this computer, opened read-only.",
    logo: "sqlite",
    fields: [{ key: "path", label: "Database file", required: true, file: true }],
    grant: "--allow-db name[:write]",
  },
  http: {
    title: "Web API",
    summary: "A credential for one host, sent only to it, over HTTPS.",
    logo: "http",
    fields: [
      { key: "host", label: "Host", placeholder: "api.example.com", required: true },
      { key: "header", label: "Header name", placeholder: "Authorization (a bearer token) or X-Api-Key", hint: "Leave empty to send a bearer token." },
    ],
    secret: { label: "Credential", hint: "The API key or token.", required: true },
    grant: "--allow-http host",
  },
  webhook: {
    title: "Webhook",
    summary: "Post signed events to an address. A person approves every send.",
    logo: "webhook",
    fields: [{ key: "url", label: "Address", placeholder: "https://hooks.example.com/zorix", required: true }],
    secret: { label: "Signing key", hint: "The receiver uses it to check events came from ZORIX. Make a long random one.", required: true },
    grant: "--allow-http host",
  },
};

