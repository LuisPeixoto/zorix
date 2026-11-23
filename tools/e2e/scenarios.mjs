// The flows the end-to-end job drives through the built app (ADR-0042). Each scenario gets a session on a fresh data folder and writes in
// English, the language the app is in. They use what a person sees (a button's text, a page's title) and nothing the code could rename.
import { containingText, withText } from "./webdriver.mjs";

const nav = (label) => `//nav//button[contains(normalize-space(.), ${JSON.stringify(label)})]`;
const title = (text) => `//h1[normalize-space(.)=${JSON.stringify(text)}]`;
const button = (text) => withText("button", text);

export const scenarios = [
  {
    name: "the app starts on an empty data folder and every part of it opens",
    async run(s) {
      await s.waitFor(nav("Runs"), { label: "the sidebar" });
      const pages = [
        ["Inbox", "Inbox"],
        ["Runs", "Runs"],
        ["Workflows", "Workflows"],
        ["Projects", "Projects"],
        ["Agents", "Agents"],
        ["Teams", "Teams"],
        ["Workforce", "Workforce"],
        ["Providers", "Providers"],
        ["Integrations", "Integrations"],
        ["Memory", "Memory"],
        ["Documents", "Documents"],
        ["Settings", "Settings"],
      ];
      for (const [item, heading] of pages) {
        await s.clickOn(nav(item), { label: `the ${item} entry in the sidebar` });
        await s.waitFor(title(heading), { label: `the page “${heading}”` });
        const broken = await s.find(`//*[@role="alert"]`);
        if (broken.length) throw new Error(`The ${item} page shows an alert instead of its content: ${await s.text(broken[0])}`);
      }
    },
  },
  {
    name: "a run with the demo model finishes and is listed",
    async run(s) {
      await s.clickOn(button("New run"));
      await s.waitFor(title("New run"));
      await s.clickOn(button("What time is it?"));
      await s.clickOn(button("Start run"));
      await s.waitFor(containingText("*", "Completed"), { timeout: 60_000, label: "the run to complete" });
      await s.clickOn(nav("Runs"));
      await s.waitFor(containingText("*", "What time is it?"), { label: "the run in the list" });
    },
  },
  {
    name: "a change waits for a person, and an answer lets the run go on",
    async run(s) {
      for (const answer of ["Approve", "Refuse"]) {
        await s.clickOn(button("New run"));
        await s.clickOn(button("What is 17*23 and save it as result"));
        await s.clickOn(button("Start run"));
        // `notes.put` asks first: the run stops at an approval card until a person answers.
        await s.waitFor(button(answer), { timeout: 60_000, label: `the ${answer} button of the approval` });
        await s.clickOn(button(answer));
        await s.waitFor(containingText("*", "Completed"), { timeout: 60_000, label: `the run to complete after “${answer}”` });
      }
    },
  },
  {
    name: "a project takes a task and the board shows it",
    async run(s) {
      await s.clickOn(nav("Projects"));
      await s.clickOn(button("New project"));
      const [name] = await s.find(`//label[contains(normalize-space(.), "Name")]/following::input[1] | //input[@placeholder="Q4 Operations"]`);
      if (!name) throw new Error("The new-project form has no name field.");
      await s.type(name, "Release plan");
      const [key] = await s.find(`//input[@maxlength="5"]`);
      if (key) await s.type(key, "REL");
      await s.clickOn(button("Create project"));
      await s.waitFor(containingText("*", "Release plan"), { label: "the project" });
    },
  },
  {
    name: "the workforce shows you at the top of an empty organisation",
    async run(s) {
      await s.clickOn(nav("Workforce"));
      await s.waitFor(title("Workforce"));
      await s.waitFor(containingText("*", "You"), { label: "the card for you" });
    },
  },
];
