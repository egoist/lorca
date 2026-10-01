// One installed plugin on a Runner, after the macOS app's PluginViewController: its state, the
// sign-in for a remote server, its variables (a secret is written, never read back), the skills it
// brought, and Remove. It also shows the Always allowed rules for its tools.

import { createSignal, For, onSettled, Show } from "solid-js";
import { host } from "../../host";
import { L, Lc } from "../../l10n";
import { pluginStateColor, type Device, type PluginDetail } from "../../model/models";
import { onStoreEvent, track } from "../../model/reactive";
import { errorText, store } from "../../model/store";
import { Button } from "../controls";
import { alert, presentSheet, Sheet } from "../overlay";
import { ActionRow, KeyValueRow, Section } from "../sections";

export function presentPlugin(pluginID: string, runner: Device): void {
  presentSheet((dismiss) => <PluginSheet pluginID={pluginID} runner={runner} dismiss={dismiss} />);
}

function hostOf(link: string | undefined): string | undefined {
  if (!link) return undefined;
  try {
    return new URL(link).host || undefined;
  } catch {
    return undefined;
  }
}

/** A sign-in code's row: the code, and a button that copies it and opens the page to enter it on. */
function CodeRow(props: { label: string; code: string; link: string }) {
  const [copied, setCopied] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  return (
    <ActionRow
      label={props.label}
      value={props.code}
      tint="var(--label)"
      monospaced
      actionTitle={copied() ? L("Copied") : L("Copy code and open %@", hostOf(props.link) ?? L("link"))}
      actionCopied={copied()}
      onAction={() => {
        void host.copyText(props.code);
        setCopied(true);
        clearTimeout(timer);
        timer = setTimeout(() => setCopied(false), 1500);
        void host.openExternal(props.link);
      }}
    />
  );
}

function PluginSheet(props: { pluginID: string; runner: Device; dismiss: () => void }) {
  const installed = props.runner.plugins.find((plugin) => plugin.id === props.pluginID);
  const name = installed?.name ?? props.pluginID;
  const [detail, setDetail] = createSignal<PluginDetail | null>(null);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);
  const fields = new Map<string, HTMLInputElement>();
  /** Only the newest load renders, so an older answer arriving late (a sealed request to another
   * Runner) never covers a newer one, such as the detail with a sign-in code. */
  let loads = 0;

  const load = async () => {
    const load = ++loads;
    try {
      const next = await store.pluginDetail(props.pluginID, props.runner.id);
      if (load !== loads) return;
      setDetail(next);
      setLoadError(null);
    } catch (error) {
      if (load !== loads) return;
      setLoadError(errorText(error));
    }
  };

  onSettled(() => {
    void load();
    // The Runner's state moved (a sign-in finished, a connection failed).
    return onStoreEvent((event) => {
      if (event.kind === "rosterChanged" || event.kind === "snapshotReplaced") void load();
    });
  });

  const prefix = `${props.pluginID}/`;
  const rules = () => {
    track.roster();
    return store.autoReview.rules.filter((rule) => rule.tool?.startsWith(prefix));
  };
  const resetRules = () => {
    const review = store.autoReview;
    store.setAutoReview({ ...review, rules: review.rules.filter((rule) => !rule.tool?.startsWith(prefix)) });
  };
  const oauthServers = () => detail()?.servers.filter((server) => server.oauth) ?? [];

  const save = async () => {
    const values: Record<string, string> = {};
    for (const variable of detail()?.variables ?? []) {
      const field = fields.get(variable.name);
      if (!field) continue;
      if (field.value !== "" || !variable.secret) values[variable.name] = field.value;
    }
    setSaving(true);
    try {
      await store.setPluginVariables(props.pluginID, props.runner.id, values);
      // A secret is written, never read back.
      for (const variable of detail()?.variables ?? []) {
        const field = fields.get(variable.name);
        if (variable.secret && field) field.value = "";
      }
      void load();
    } catch (error) {
      void alert({ message: L("Couldn't save"), informative: errorText(error) });
    } finally {
      setSaving(false);
    }
  };

  const connect = async () => {
    try {
      // The Runner notes the sign-in on the plugin, so the State row reads it.
      await store.connectPlugin(props.pluginID, props.runner.id);
      void load();
    } catch (error) {
      void alert({ message: L("Couldn't start the sign-in"), informative: errorText(error) });
    }
  };

  const confirmRemove = async () => {
    const answer = await alert({
      message: L("Remove %@ from %@?", name, props.runner.name),
      informative: L("Every bot on %@ loses it, and its keys and sign-ins there are forgotten.", props.runner.name),
      style: "warning",
      buttons: [{ title: L("Remove") }, { title: L("Cancel") }],
    });
    if (answer !== 0) return;
    try {
      await store.uninstallPlugin(props.pluginID, props.runner.id);
      props.dismiss();
    } catch (error) {
      void alert({ message: L("Couldn't remove it"), informative: errorText(error) });
    }
  };

  return (
    <Sheet
      title={name}
      subtitle={[installed?.description ?? "", L("Installed on %@.", props.runner.name)].filter((part) => part !== "").join(" ")}
      width={520}
      confirm={L("Done")}
      cancel={null}
      onConfirm={props.dismiss}
      onCancel={props.dismiss}
    >
      <Section title={L("Status")}>
        <Show
          when={!loadError() && detail()}
          fallback={<KeyValueRow label={L("State")} value={loadError() ?? L("Loading…")} tint={loadError() ? "var(--red)" : "var(--label-2)"} />}
        >
          {(current) => (
            <>
              <KeyValueRow label={L("State")} value={current().status.detail} tint={pluginStateColor(current().status.state)} />
              <Show when={rules().length > 0}>
                <ActionRow
                  label={Lc("Always allowed", "plugin tools")}
                  value={rules()
                    .map((rule) => (rule.tool ?? "").slice(prefix.length))
                    .join(", ")}
                  tint="var(--label)"
                  actionTitle={L("Reset")}
                  onAction={resetRules}
                />
              </Show>
              <Show when={current().homepage && hostOf(current().homepage)}>
                {(site) => (
                  <ActionRow
                    label={L("Site")}
                    value={site()}
                    tint="var(--label-2)"
                    actionTitle={L("Open")}
                    onAction={() => void host.openExternal(current().homepage!)}
                  />
                )}
              </Show>
            </>
          )}
        </Show>
      </Section>
      <Show when={oauthServers().length > 0}>
        <Section title={L("Sign-in")}>
          <For each={oauthServers()} keyed={(server) => server.name}>
            {(server) => {
              const label = () => (oauthServers().length > 1 ? server().name : L("Account"));
              // A device-flow sign-in waits for its code: the code, and a button that copies it and
              // opens the page to enter it on.
              return (
                <Show
                  when={server().code !== undefined && server().link !== undefined}
                  fallback={
                    <ActionRow
                      label={label()}
                      value={server().signedIn ? L("Signed in") : L("Not signed in")}
                      tint={server().signedIn ? "var(--green)" : "var(--label-2)"}
                      actionTitle={server().signedIn ? L("Sign in again") : L("Sign in")}
                      onAction={() => void connect()}
                    />
                  }
                >
                  <CodeRow label={label()} code={server().code!} link={server().link!} />
                </Show>
              );
            }}
          </For>
        </Section>
      </Show>
      <Show when={(detail()?.variables.length ?? 0) > 0}>
        <Section title={Lc("Setup", "plugin variables")}>
          <For each={detail()?.variables ?? []} keyed={(variable) => variable.name}>
            {(variable) => (
              <div class="row field-row" data-label={variable().name}>
                <span class="row-key mono">{variable().name + (variable().required ? " *" : "")}</span>
                <input
                  ref={(element) => fields.set(variable().name, element)}
                  class="text-field mono small field-row-input"
                  type={variable().secret ? "password" : "text"}
                  value={variable().secret ? "" : (variable().value ?? "")}
                  placeholder={variable().secret ? (variable().isSet ? L("Set · type to replace") : L("Not set")) : variable().isSet ? "" : L("Not set")}
                  title={variable().description}
                  spellcheck="false"
                  autocomplete="off"
                />
              </div>
            )}
          </For>
        </Section>
      </Show>
      <Show when={(detail()?.skills.length ?? 0) > 0}>
        <Section title={L("Skills")}>
          <For each={detail()?.skills ?? []}>{(skill) => <KeyValueRow label={skill.name} value={skill.description} />}</For>
        </Section>
      </Show>
      <div class="sheet-note secondary">
        {props.runner.isThisDevice
          ? L("Keys and sign-ins stay on this device.")
          : L("Keys and sign-ins are sent sealed to %@ and stay there.", props.runner.name)}
      </div>
      <div class="sheet-actions">
        <Show when={(detail()?.variables.length ?? 0) > 0}>
          <Button disabled={saving()} onClick={() => void save()}>
            {L("Save")}
          </Button>
        </Show>
        <span class="sheet-spacer" />
        <Button onClick={() => void confirmRemove()}>{L("Remove…")}</Button>
      </div>
    </Sheet>
  );
}
