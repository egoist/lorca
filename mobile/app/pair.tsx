import { CameraView, useCameraPermissions } from "expo-camera";
import * as Clipboard from "expo-clipboard";
import * as Haptics from "expo-haptics";
import { LinearGradient } from "expo-linear-gradient";
import { useEffect, useRef, useState } from "react";
import { useLocalSearchParams } from "expo-router";
import { ActivityIndicator, Alert, KeyboardAvoidingView, Modal, Platform, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { engine } from "../src/core/engine";
import { hostFacts } from "../src/core/host";
import { parsePairingString } from "../src/core/pairing";
import { Symbol } from "../src/ui/Symbol";
import { Font, usePalette } from "../src/ui/theme";

type Phase = "idle" | "posting" | "waiting";

export default function PairScreen() {
  const p = usePalette();
  const insets = useSafeAreaInsets();
  const [code, setCode] = useState("");
  const [name, setName] = useState(() => hostFacts().name);
  const [phase, setPhase] = useState<Phase>("idle");
  const [scanning, setScanning] = useState(false);
  const [permission, requestPermission] = useCameraPermissions();
  const cancel = useRef<AbortController | null>(null);
  /// Set the moment a pairing starts. The camera reports the same code on every frame until
  /// it unmounts, and state updates land later than the next frame, so a ref is what keeps a
  /// second request (which the relay refuses as "already has a request") from going out.
  const inFlight = useRef(false);
  const busy = phase !== "idle";
  // A pairing code opened as a link (`tinybot://pair?…`, from the Camera app or a tap on the
  // Mac's code) lands here with its fields as params: pair with it right away.
  const params = useLocalSearchParams<{ relay?: string; id?: string; ek?: string; n?: string }>();
  useEffect(() => {
    if (!params.relay || !params.id || !params.ek || !params.n || inFlight.current) return;
    const text = `tinybot://pair?relay=${encodeURIComponent(params.relay)}&id=${params.id}&ek=${params.ek}&n=${params.n}`;
    setCode(text);
    void pair(text);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params.relay, params.id, params.ek, params.n]);

  async function pair(text: string) {
    if (inFlight.current) return;
    try {
      parsePairingString(text);
    } catch (error) {
      Alert.alert("Not a pairing code", error instanceof Error ? error.message : String(error));
      return;
    }
    inFlight.current = true;
    cancel.current = new AbortController();
    setPhase("posting");
    try {
      await engine.pair(text, name, (progress) => setPhase(progress.phase === "done" ? "idle" : progress.phase), cancel.current.signal);
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success);
    } catch (error) {
      setPhase("idle");
      // Cancel was tapped here: the core's "Pairing cancelled" is not news.
      if (cancel.current?.signal.aborted) return;
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error);
      Alert.alert("Pairing failed", error instanceof Error ? error.message : String(error));
    } finally {
      inFlight.current = false;
    }
  }

  async function scan() {
    if (!permission?.granted) {
      const result = await requestPermission();
      if (!result.granted) {
        Alert.alert("Camera access needed", "Allow the camera to scan the pairing code, or paste the code instead.");
        return;
      }
    }
    setScanning(true);
  }

  async function paste() {
    const text = (await Clipboard.getStringAsync()).trim();
    if (text.includes("pair?")) {
      setCode(text);
      void pair(text);
    } else {
      Alert.alert("Nothing to paste", "Copy the pairing code from Tinybot on your Mac first (Devices › Pair a Device).");
    }
  }

  return (
    <LinearGradient colors={p.dark ? ["#1a1b3a", "#0b0c1a"] : ["#eef1ff", "#ffffff"]} style={{ flex: 1 }}>
      <KeyboardAvoidingView behavior={Platform.OS === "ios" ? "padding" : undefined} style={{ flex: 1 }}>
        <ScrollView contentContainerStyle={[styles.content, { paddingTop: insets.top + 56, paddingBottom: insets.bottom + 24 }]} keyboardShouldPersistTaps="handled" keyboardDismissMode="on-drag">
          <View style={styles.logoWrap}>
            <LinearGradient colors={["#6b66f5", "#3d85f0", "#2eb3dc"]} style={styles.logo}>
              <View style={styles.face}>
                <View style={styles.eye} />
                <View style={styles.eye} />
              </View>
            </LinearGradient>
          </View>
          <Text style={[styles.title, { color: p.label }]}>Pair with your Mac</Text>
          <Text style={[styles.subtitle, { color: p.secondaryLabel }]}>
            Your bots run on your own machines. This phone joins them with a code from Tinybot on your Mac: open Devices, choose Pair a Device, and scan or copy the code.
          </Text>

          <View style={[styles.card, { backgroundColor: p.cell }]}>
            <Text style={[styles.label, { color: p.secondaryLabel }]}>THIS PHONE</Text>
            <TextInput value={name} onChangeText={setName} placeholder="iPhone" placeholderTextColor={p.tertiaryLabel} style={[styles.input, { color: p.label }]} autoCapitalize="words" editable={!busy} />
          </View>

          {busy ? (
            <View style={[styles.card, styles.progress, { backgroundColor: p.cell }]}>
              <ActivityIndicator />
              <Text style={{ color: p.label, fontSize: Font.body }}>{phase === "posting" ? "Sending the request…" : "Waiting for your Mac to accept…"}</Text>
              <Pressable onPress={() => cancel.current?.abort()} hitSlop={10}>
                <Text style={{ color: p.tint, fontSize: Font.body }}>Cancel</Text>
              </Pressable>
            </View>
          ) : (
            <>
              <Pressable onPress={scan} style={({ pressed }) => [styles.primary, { backgroundColor: p.tint, opacity: pressed ? 0.85 : 1 }]}>
                <Symbol name="qrcode.viewfinder" size={20} color="#FFFFFF" weight="semibold" />
                <Text style={styles.primaryText}>Scan Code</Text>
              </Pressable>
              <Pressable onPress={paste} style={({ pressed }) => [styles.secondary, { backgroundColor: p.fill, opacity: pressed ? 0.7 : 1 }]}>
                <Symbol name="doc.on.clipboard" size={18} color={p.tint} />
                <Text style={[styles.secondaryText, { color: p.tint }]}>Paste Code</Text>
              </Pressable>
              <View style={[styles.card, { backgroundColor: p.cell, marginTop: 8 }]}>
                <Text style={[styles.label, { color: p.secondaryLabel }]}>OR TYPE IT</Text>
                <TextInput
                  value={code}
                  onChangeText={setCode}
                  placeholder="tinybot://pair?relay=…"
                  placeholderTextColor={p.tertiaryLabel}
                  style={[styles.input, styles.codeInput, { color: p.label }]}
                  autoCapitalize="none"
                  autoCorrect={false}
                  multiline
                  onSubmitEditing={() => pair(code)}
                />
                <Pressable onPress={() => pair(code)} disabled={!code.trim()} hitSlop={8} style={{ alignSelf: "flex-end", marginTop: 6 }}>
                  <Text style={{ color: code.trim() ? p.tint : p.tertiaryLabel, fontSize: Font.body, fontWeight: "600" }}>Pair</Text>
                </Pressable>
              </View>
            </>
          )}
        </ScrollView>
      </KeyboardAvoidingView>

      <Modal visible={scanning} animationType="slide" presentationStyle="fullScreen" onRequestClose={() => setScanning(false)}>
        <View style={{ flex: 1, backgroundColor: "#000" }}>
          <CameraView
            style={StyleSheet.absoluteFill}
            facing="back"
            barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
            onBarcodeScanned={({ data }) => {
              if (!data.includes("pair?") || inFlight.current) return;
              setScanning(false);
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
              setCode(data);
              void pair(data);
            }}
          />
          <View style={[styles.scanOverlay, { paddingTop: insets.top + 12 }]}>
            <Pressable onPress={() => setScanning(false)} style={styles.close} hitSlop={10} accessibilityLabel="Close">
              <Symbol name="xmark" size={16} color="#FFFFFF" weight="bold" />
            </Pressable>
            <Text style={styles.scanHint}>Point at the pairing code on your Mac</Text>
          </View>
          <View pointerEvents="none" style={styles.reticleWrap}>
            <View style={styles.reticle} />
          </View>
        </View>
      </Modal>
    </LinearGradient>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: 24, gap: 12 },
  logoWrap: { alignItems: "center", marginBottom: 8 },
  logo: { width: 88, height: 88, borderRadius: 24, alignItems: "center", justifyContent: "center" },
  face: { width: 48, height: 38, borderRadius: 14, backgroundColor: "#fff", flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 10 },
  eye: { width: 8, height: 8, borderRadius: 4, backgroundColor: "#4576f2" },
  title: { fontSize: 30, fontWeight: "700", textAlign: "center", letterSpacing: -0.4 },
  subtitle: { fontSize: 16, lineHeight: 22, textAlign: "center", marginBottom: 12 },
  card: { borderRadius: 14, padding: 14, gap: 6 },
  label: { fontSize: 12, fontWeight: "600", letterSpacing: 0.4 },
  input: { fontSize: Font.body, paddingVertical: 4 },
  codeInput: { minHeight: 56, fontFamily: Platform.select({ ios: "Menlo", default: "monospace" }), fontSize: 13 },
  primary: { flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8, borderRadius: 14, paddingVertical: 15 },
  primaryText: { color: "#fff", fontSize: Font.body, fontWeight: "600" },
  secondary: { flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8, borderRadius: 14, paddingVertical: 14 },
  secondaryText: { fontSize: Font.body, fontWeight: "600" },
  progress: { flexDirection: "row", alignItems: "center", gap: 12 },
  scanOverlay: { position: "absolute", top: 0, left: 0, right: 0, paddingHorizontal: 16, flexDirection: "row", alignItems: "center", gap: 12 },
  close: { width: 36, height: 36, borderRadius: 18, backgroundColor: "rgba(0,0,0,0.5)", alignItems: "center", justifyContent: "center" },
  scanHint: { color: "#fff", fontSize: 15, fontWeight: "500", flex: 1 },
  reticleWrap: { position: "absolute", top: 0, left: 0, right: 0, bottom: 0, alignItems: "center", justifyContent: "center" },
  reticle: { width: 240, height: 240, borderRadius: 24, borderWidth: 2, borderColor: "rgba(255,255,255,0.8)" },
});
