// What each kind of connection asks for. The interface only collects it; the rules live in the hub (Rust).
import type { ConnectionKind } from "../../api.ts";
import { t } from "../../i18n/index.ts";

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
    title: t("common.github"),
    summary: t("integrations.kinds.issuesPullRequestsAnd"),
    logo: "github",
    fields: [{ key: "base_url", label: t("integrations.kinds.apiAddress"), placeholder: "https://api.github.com", hint: t("integrations.kinds.onlyForGithubEnterprise") }],
    secret: { label: t("integrations.kinds.personalAccessToken"), hint: t("integrations.kinds.leaveEmptyToRead"), required: false },
    grant: "--allow-repo owner/repo[:write]",
  },
  gitlab: {
    title: t("integrations.kinds.gitlab"),
    summary: t("integrations.kinds.issuesMergeRequestsAnd"),
    logo: "gitlab",
    fields: [{ key: "base_url", label: t("common.address"), placeholder: "https://gitlab.com", hint: t("integrations.kinds.onlyForYourOwn") }],
    secret: { label: t("integrations.kinds.accessToken"), hint: t("integrations.kinds.leaveEmptyToRead2"), required: false },
    grant: "--allow-repo gitlab:group/project[:write]",
  },
  jira: {
    title: t("integrations.kinds.jira"),
    summary: t("integrations.kinds.searchReadCreateComment"),
    logo: "jira",
    fields: [
      { key: "base_url", label: t("integrations.kinds.siteAddress"), placeholder: "https://acme.atlassian.net", required: true },
      { key: "email", label: t("integrations.kinds.accountEmail"), required: true },
    ],
    secret: { label: t("integrations.kinds.apiToken"), hint: t("integrations.kinds.madeAtIdAtlassian"), required: true },
    grant: "--allow-jira KEY[:write]",
  },
  slack: {
    title: t("common.slack"),
    summary: t("integrations.kinds.readChannelsAndSend"),
    logo: "slack",
    fields: [],
    secret: { label: t("integrations.kinds.botToken"), hint: t("integrations.kinds.startsWithXoxbMade"), required: true },
    grant: "--allow-chat slack:#channel[:write]",
  },
  discord: {
    title: t("common.discord"),
    summary: t("integrations.kinds.readChannelsAndSend2"),
    logo: "discord",
    fields: [],
    secret: { label: t("integrations.kinds.botToken"), hint: t("integrations.kinds.madeInTheDiscord"), required: true },
    grant: "--allow-chat discord:<channel id>[:write]",
  },
  postgres: {
    title: t("integrations.kinds.postgres"),
    summary: t("integrations.kinds.queryADatabaseIn"),
    logo: "postgres",
    fields: [
      { key: "host", label: t("integrations.kinds.host"), required: true },
      { key: "port", label: t("integrations.kinds.port"), placeholder: "5432", number: true },
      { key: "database", label: t("integrations.kinds.database"), required: true },
      { key: "user", label: t("integrations.kinds.user"), required: true },
    ],
    secret: { label: t("integrations.kinds.password"), hint: t("integrations.kinds.leaveEmptyForTrust"), required: false },
    grant: "--allow-db name[:write]",
  },
  sqlite: {
    title: "SQLite",
    summary: t("integrations.kinds.readADatabaseFile"),
    logo: "sqlite",
    fields: [{ key: "path", label: t("integrations.kinds.databaseFile"), required: true, file: true }],
    grant: "--allow-db name[:write]",
  },
  http: {
    title: t("integrations.kinds.webApi"),
    summary: t("integrations.kinds.aCredentialForOne"),
    logo: "http",
    fields: [
      { key: "host", label: t("integrations.kinds.host"), placeholder: "api.example.com", required: true },
      { key: "header", label: t("integrations.kinds.headerName"), placeholder: t("integrations.kinds.authorizationABearerToken"), hint: t("integrations.kinds.leaveEmptyToSend") },
    ],
    secret: { label: t("integrations.kinds.credential"), hint: t("integrations.kinds.theApiKeyOr"), required: true },
    grant: "--allow-http host",
  },
  webhook: {
    title: t("integrations.kinds.webhook"),
    summary: t("integrations.kinds.postSignedEventsTo"),
    logo: "webhook",
    fields: [{ key: "url", label: t("common.address"), placeholder: "https://hooks.example.com/zorix", required: true }],
    secret: { label: t("integrations.kinds.signingKey"), hint: t("integrations.kinds.theReceiverUsesIt"), required: true },
    grant: "--allow-http host",
  },
};

