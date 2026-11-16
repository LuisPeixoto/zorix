import "./early.ts"; // first: sets the theme before anything is drawn
import "@fontsource-variable/inter";
import "@fontsource-variable/jetbrains-mono";
import { mount } from "svelte";
import App from "./App.svelte";
import "./app.css";

const app = mount(App, { target: document.getElementById("app")! });

export default app;
