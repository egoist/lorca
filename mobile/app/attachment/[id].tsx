// A photo from the transcript, full screen on black.

import { Image } from "expo-image";
import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { StyleSheet, View } from "react-native";
import { useStore } from "../../src/core/store";

export default function AttachmentScreen() {
  const { id, name } = useLocalSearchParams<{ id: string; name?: string }>();
  const router = useRouter();
  const uri = useStore((s) => s.files[id]);
  return (
    <View style={styles.screen}>
      <Stack.Screen options={{ title: name ?? "Photo" }} />
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button icon="xmark" accessibilityLabel="Close" onPress={() => router.back()} />
      </Stack.Toolbar>
      {uri && <Image source={{ uri }} style={styles.image} contentFit="contain" />}
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: "#000000" },
  image: { flex: 1 },
});
