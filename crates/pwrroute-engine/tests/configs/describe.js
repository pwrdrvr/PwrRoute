// What Engine::describe makes of a browser that opens nothing and a
// function QuickJS has no source for.
export default {
  defaultBrowser: null,
  handlers: [
    { match: RegExp.prototype.test.bind(/bound\.example/), browser: "Safari" },
    { match: "none.example/*", browser: null },
  ],
};
