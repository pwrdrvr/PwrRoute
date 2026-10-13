import { resolve } from "node:path";
import react from "@vitejs/plugin-react";
import { defineConfig, externalizeDepsPlugin } from "electron-vite";

export default defineConfig(({ command }) => {
  const productionDefine = command === "build"
    ? { "process.env.NODE_ENV": JSON.stringify("production") }
    : {};

  return {
    main: {
      define: productionDefine,
      // Source-form workspace packages get bundled, not externalized — Node's
      // ESM resolver can't follow extensionless imports inside source-form
      // packages. Mirrors PwrGit / PwrSnap / PwrAgnt.
      plugins: [
        externalizeDepsPlugin({
          exclude: ["@pwrroute/shared", "@pwrroute/mcp-server"]
        })
      ],
      build: {
        minify: "esbuild",
        sourcemap: false,
        rollupOptions: {
          input: { index: resolve(__dirname, "src/main/index.ts") },
          output: { entryFileNames: "[name].js" }
        }
      }
    },
    preload: {
      define: productionDefine,
      plugins: [externalizeDepsPlugin({ exclude: ["@pwrroute/shared"] })],
      build: {
        minify: "esbuild",
        sourcemap: false,
        // CJS so the sandboxed preload can `require("electron")`. `.cjs`
        // extension is required because the package is `"type": "module"`.
        rollupOptions: { output: { format: "cjs", entryFileNames: "index.cjs" } }
      }
    },
    renderer: {
      plugins: [react()],
      resolve: {
        alias: { "@renderer": resolve(__dirname, "src/renderer/src") }
      },
      build: { minify: "esbuild", sourcemap: false }
    }
  };
});
