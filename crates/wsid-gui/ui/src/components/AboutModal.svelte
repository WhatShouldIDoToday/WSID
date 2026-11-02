<script lang="ts">
  import { onMount } from "svelte";
  import frontendLicensesJson from "../lib/generated-frontend-licenses.json";
  import {
    getAboutInfo,
    type AboutMetadata,
    type FrontendDependencyLicense,
    type LicenseGroup,
  } from "../lib/ipc";

  interface Props {
    onClose: () => void;
  }

  let { onClose }: Props = $props();

  let aboutData = $state<AboutMetadata | null>(null);
  let loading = $state(true);
  let errorMsg = $state<string | null>(null);
  let searchFilter = $state("");
  let activeSection = $state<"rust" | "frontend" | "project">("rust");

  const frontendPackages: FrontendDependencyLicense[] =
    frontendLicensesJson as FrontendDependencyLicense[];

  let filteredRustGroups = $derived.by((): LicenseGroup[] => {
    if (!aboutData) return [];
    const q = searchFilter.trim().toLowerCase();
    if (!q) return aboutData.rust_licenses.licenses;

    return aboutData.rust_licenses.licenses
      .map((group) => {
        const groupMatches =
          group.id.toLowerCase().includes(q) ||
          group.name.toLowerCase().includes(q);
        const matchingCrates = group.used_by.filter(
          (c) =>
            c.name.toLowerCase().includes(q) ||
            c.version.toLowerCase().includes(q) ||
            (c.repository && c.repository.toLowerCase().includes(q)) ||
            c.authors.some((a) => a.toLowerCase().includes(q)),
        );
        if (groupMatches) {
          return group;
        }
        if (matchingCrates.length > 0) {
          return {
            ...group,
            used_by: matchingCrates,
          };
        }
        return null;
      })
      .filter((g): g is LicenseGroup => g !== null);
  });

  let filteredFrontendPackages = $derived.by(
    (): FrontendDependencyLicense[] => {
      const q = searchFilter.trim().toLowerCase();
      if (!q) return frontendPackages;
      return frontendPackages.filter(
        (pkg) =>
          pkg.name.toLowerCase().includes(q) ||
          (pkg.license && pkg.license.toLowerCase().includes(q)) ||
          (pkg.repository && pkg.repository.toLowerCase().includes(q)) ||
          (pkg.author && pkg.author.toLowerCase().includes(q)),
      );
    },
  );

  let totalRustCrateAttributions = $derived.by((): number => {
    if (!aboutData) return 0;
    return aboutData.rust_licenses.overview.reduce(
      (sum, item) => sum + item.count,
      0,
    );
  });

  onMount(() => {
    void (async () => {
      loading = true;
      errorMsg = null;
      try {
        aboutData = await getAboutInfo();
      } catch (err) {
        errorMsg = String(err);
      } finally {
        loading = false;
      }
    })();
  });

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      onClose();
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div
  class="drawer-backdrop"
  role="dialog"
  aria-modal="true"
  aria-labelledby="about-modal-title"
  tabindex="-1"
  onclick={(e) => {
    if (e.target === e.currentTarget) onClose();
  }}
  onkeydown={(e) => {
    if (e.key === "Escape") onClose();
  }}
>
  <div class="drawer-panel about-modal-panel">
    <header class="drawer-header">
      <div>
        <div
          style="display: flex; align-items: center; gap: 0.6rem; flex-wrap: wrap;"
        >
          <span class="task-id-badge">WSID</span>
          <span class="badge badge-q2">
            v{aboutData?.version ?? "0.1.0"}
          </span>
          <span class="badge badge-q1">
            License: {aboutData?.license_spdx ?? "Apache-2.0"}
          </span>
        </div>
        <h2
          id="about-modal-title"
          style="margin: 0.4rem 0 0 0; font-size: 1.15rem;"
        >
          {aboutData?.app_full_name ?? "WSID — What Should I Do?"}
        </h2>
      </div>
      <button
        type="button"
        class="btn btn-sm"
        onclick={onClose}
        aria-label="Close About and Licenses dialog (Escape)"
      >
        ✕ Close (Esc)
      </button>
    </header>

    <div class="drawer-body">
      <section class="card" aria-label="Application Metadata">
        <div
          style="display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 0.75rem;"
        >
          <div>
            <p style="margin: 0 0 0.25rem 0; font-size: 0.9rem;">
              Offline-first, cross-platform desktop &amp; terminal task manager
              with human + AI persona collaboration.
            </p>
            <p
              style="margin: 0; font-size: 0.82rem; color: var(--text-secondary);"
            >
              Repository:{" "}
              <a
                href={aboutData?.repository_url ??
                  "https://github.com/dmbeta/WSID"}
                target="_blank"
                rel="noopener noreferrer"
                style="color: var(--accent); font-family: var(--font-mono);"
              >
                {aboutData?.repository_url ?? "https://github.com/dmbeta/WSID"}
              </a>
            </p>
          </div>
          <div
            style="display: flex; gap: 0.4rem; flex-wrap: wrap;"
            role="tablist"
            aria-label="License categories"
          >
            <button
              type="button"
              role="tab"
              aria-selected={activeSection === "rust"}
              class="btn btn-sm {activeSection === 'rust' ? 'btn-primary' : ''}"
              onclick={() => (activeSection = "rust")}
            >
              Rust Crates ({totalRustCrateAttributions})
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={activeSection === "frontend"}
              class="btn btn-sm {activeSection === 'frontend'
                ? 'btn-primary'
                : ''}"
              onclick={() => (activeSection = "frontend")}
            >
              Frontend Packages ({frontendPackages.length})
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={activeSection === "project"}
              class="btn btn-sm {activeSection === 'project'
                ? 'btn-primary'
                : ''}"
              onclick={() => (activeSection = "project")}
            >
              Project License (Apache-2.0)
            </button>
          </div>
        </div>
      </section>

      {#if loading}
        <p role="status">Loading license and attribution metadata…</p>
      {:else if errorMsg}
        <div
          role="alert"
          style="padding: 0.75rem; border: 1px solid var(--danger); border-radius: var(--radius-sm); color: var(--danger);"
        >
          {errorMsg}
        </div>
      {:else if aboutData}
        {#if activeSection === "project"}
          <section class="card" aria-labelledby="project-license-heading">
            <h3
              id="project-license-heading"
              style="margin: 0 0 0.5rem 0; font-size: 0.95rem;"
            >
              WSID Project License ({aboutData.license_spdx})
            </h3>
            <pre class="license-text-block">{aboutData.license_text}</pre>
          </section>
        {:else}
          <div class="field-group">
            <label class="field-label" for="license-search-input">
              Search Third-Party Packages, Licenses, Authors, or Repositories
            </label>
            <input
              id="license-search-input"
              type="search"
              placeholder="Filter by package name (e.g. ratatui, tauri, svelte), SPDX id (MIT, Apache-2.0, MPL-2.0)..."
              bind:value={searchFilter}
            />
          </div>

          {#if activeSection === "rust"}
            <section class="card" aria-labelledby="rust-overview-heading">
              <h3
                id="rust-overview-heading"
                style="margin: 0 0 0.5rem 0; font-size: 0.9rem;"
              >
                Rust Crate License Summary (generated by cargo-about)
              </h3>
              <div style="display: flex; flex-wrap: wrap; gap: 0.4rem;">
                {#each aboutData.rust_licenses.overview as item (item.id)}
                  <button
                    type="button"
                    class="badge"
                    style="cursor: pointer;"
                    onclick={() => (searchFilter = item.id)}
                    title="Filter by {item.name}"
                  >
                    {item.id}: {item.count}
                  </button>
                {/each}
                {#if searchFilter}
                  <button
                    type="button"
                    class="btn btn-sm"
                    onclick={() => (searchFilter = "")}
                  >
                    Clear Filter
                  </button>
                {/if}
              </div>
            </section>

            <section
              aria-label="Rust Third-Party License Groups"
              style="display: flex; flex-direction: column; gap: 0.65rem;"
            >
              {#if filteredRustGroups.length === 0}
                <p style="color: var(--text-muted); font-size: 0.85rem;">
                  No Rust crates or licenses match "{searchFilter}".
                </p>
              {:else}
                {#each filteredRustGroups as group, idx (`${group.id}-${idx}`)}
                  <details class="card license-details">
                    <summary class="license-summary">
                      <div>
                        <span
                          class="badge badge-q2"
                          style="margin-right: 0.5rem;">{group.id}</span
                        >
                        <strong>{group.name}</strong>
                      </div>
                      <span
                        style="font-size: 0.8rem; color: var(--text-secondary);"
                      >
                        {group.used_by.length} crate{group.used_by.length === 1
                          ? ""
                          : "s"}
                      </span>
                    </summary>

                    <div
                      style="margin-top: 0.75rem; display: flex; flex-direction: column; gap: 0.5rem;"
                    >
                      <ul
                        class="crate-attribution-list"
                        aria-label="Crates using {group.name}"
                      >
                        {#each group.used_by as krate (`${krate.name}-${krate.version}`)}
                          <li class="crate-attribution-item">
                            <div>
                              <span
                                style="font-family: var(--font-mono); font-weight: 600;"
                              >
                                {krate.name}
                              </span>
                              <span
                                style="font-family: var(--font-mono); font-size: 0.78rem; color: var(--text-secondary);"
                              >
                                v{krate.version}
                              </span>
                              {#if krate.authors.length > 0}
                                <span
                                  style="font-size: 0.76rem; color: var(--text-muted); margin-left: 0.35rem;"
                                >
                                  by {krate.authors.join(", ")}
                                </span>
                              {/if}
                            </div>
                            {#if krate.repository}
                              <a
                                href={krate.repository}
                                target="_blank"
                                rel="noopener noreferrer"
                                style="font-family: var(--font-mono); font-size: 0.75rem; color: var(--accent);"
                              >
                                {krate.repository}
                              </a>
                            {/if}
                          </li>
                        {/each}
                      </ul>

                      <details style="margin-top: 0.35rem;">
                        <summary
                          style="cursor: pointer; font-size: 0.8rem; color: var(--accent);"
                        >
                          View full {group.id} license text
                        </summary>
                        <pre class="license-text-block">{group.text}</pre>
                      </details>
                    </div>
                  </details>
                {/each}
              {/if}
            </section>
          {:else if activeSection === "frontend"}
            <section
              aria-label="Frontend Third-Party Packages"
              style="display: flex; flex-direction: column; gap: 0.65rem;"
            >
              {#if filteredFrontendPackages.length === 0}
                <p style="color: var(--text-muted); font-size: 0.85rem;">
                  No frontend packages match "{searchFilter}".
                </p>
              {:else}
                {#each filteredFrontendPackages as pkg (`${pkg.name}-${pkg.version}`)}
                  <details class="card license-details">
                    <summary class="license-summary">
                      <div>
                        <span
                          style="font-family: var(--font-mono); font-weight: 700;"
                        >
                          {pkg.name}
                        </span>
                        {#if pkg.version}
                          <span
                            style="font-family: var(--font-mono); font-size: 0.8rem; color: var(--text-secondary); margin-left: 0.35rem;"
                          >
                            v{pkg.version}
                          </span>
                        {/if}
                      </div>
                      <span class="badge badge-q2"
                        >{pkg.license ?? "Unknown"}</span
                      >
                    </summary>

                    <div
                      style="margin-top: 0.65rem; display: flex; flex-direction: column; gap: 0.4rem;"
                    >
                      {#if pkg.author}
                        <div
                          style="font-size: 0.8rem; color: var(--text-secondary);"
                        >
                          Author: {pkg.author}
                        </div>
                      {/if}
                      {#if pkg.repository}
                        <div style="font-size: 0.8rem;">
                          Repository:{" "}
                          <a
                            href={pkg.repository.replace(/^git\+/, "")}
                            target="_blank"
                            rel="noopener noreferrer"
                            style="font-family: var(--font-mono); color: var(--accent);"
                          >
                            {pkg.repository.replace(/^git\+/, "")}
                          </a>
                        </div>
                      {/if}
                      {#if pkg.noticeText}
                        <pre class="license-text-block">{pkg.noticeText}</pre>
                      {/if}
                      {#if pkg.licenseText}
                        <pre class="license-text-block">{pkg.licenseText}</pre>
                      {/if}
                    </div>
                  </details>
                {/each}
              {/if}
            </section>
          {/if}
        {/if}
      {/if}
    </div>
  </div>
</div>

<style>
  .about-modal-panel {
    width: min(860px, 95vw);
  }
  .license-details {
    padding: 0.7rem 0.9rem;
  }
  .license-summary {
    display: flex;
    justify-content: space-between;
    align-items: center;
    cursor: pointer;
    gap: 0.75rem;
    font-size: 0.88rem;
  }
  .crate-attribution-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    max-height: 220px;
    overflow-y: auto;
  }
  .crate-attribution-item {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 0.5rem;
    padding: 0.3rem 0.5rem;
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
  }
  .license-text-block {
    margin: 0.4rem 0 0 0;
    padding: 0.75rem;
    background: var(--bg-base);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: 0.75rem;
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 320px;
    overflow-y: auto;
    color: var(--text-secondary);
  }
</style>
