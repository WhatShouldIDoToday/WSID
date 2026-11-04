import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import license from "rollup-plugin-license";
import path from "node:path";

export default defineConfig({
  plugins: [
    svelte(),
    license({
      thirdParty: {
        multipleVersions: true,
        output: {
          file: path.resolve(
            __dirname,
            "src/lib/generated-frontend-licenses.json",
          ),
          template(dependencies) {
            const mapped = dependencies.map((dep) => {
              const repo =
                typeof dep.repository === "string"
                  ? dep.repository
                  : dep.repository && typeof dep.repository === "object"
                    ? dep.repository.url
                    : (dep.homepage ?? null);
              return {
                name: dep.name ?? "unknown",
                version: dep.version ?? "unknown",
                license: dep.license ?? "UNKNOWN",
                repository: repo,
                author: dep.author ? dep.author.text() : null,
                licenseText: dep.licenseText ?? "",
                noticeText: dep.noticeText ?? null,
              };
            });
            return JSON.stringify(mapped, null, 2);
          },
        },
      },
    }),
  ],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
