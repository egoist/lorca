// The answer to a coding agent's questions: a bottom sheet with a section for each question, its
// header, its text, its options with their descriptions to pick one of, or several where the
// question takes several, and an Other field for the user's own words; then Submit, enabled once
// every question has an answer, and Skip. `@expo/ui`'s sheet on both platforms, SwiftUI's on iOS
// and Material's on Android, as the command's `AnswerSheet`.

import { Button as ComposeButton, Checkbox, Column, Host as ComposeHost, ListItem, ModalBottomSheet, OutlinedTextField, RadioButton, Row, Text as ComposeText, TextButton, type TextFieldRef as ComposeFieldRef } from "@expo/ui/jetpack-compose";
import { fillMaxWidth, padding as composePadding, selectable, verticalScroll } from "@expo/ui/jetpack-compose/modifiers";
import { BottomSheet, Button as SwiftButton, Form, Group, HStack, Host as SwiftHost, Image as SwiftImage, Section, Spacer, Text as SwiftText, TextField, VStack, type TextFieldRef } from "@expo/ui/swift-ui";
import { accessibilityAddTraits, disabled, font, foregroundStyle, presentationDetents, presentationDragIndicator } from "@expo/ui/swift-ui/modifiers";
import { useRef, useState } from "react";
import { Platform, StyleSheet, type ColorValue } from "react-native";
import { pickOption, questionAnswers, writeOther, type Question, type QuestionAsk, type QuestionPick } from "../core/model";
import { t, useLanguage } from "../i18n";
import { usePalette } from "./theme";

const NO_PICK: QuestionPick = { picked: [], other: "" };

/// Up while mounted. `onSubmit` gets the answers by question id and `onSkip` declines them; each
/// rejects with why it did not go, and the sheet shows it and stays.
export function QuestionSheet({ ask, onSubmit, onSkip, onDismiss }: { ask: QuestionAsk; onSubmit: (answers: Record<string, string>) => Promise<void>; onSkip: () => Promise<void>; onDismiss: () => void }) {
  useLanguage();
  const p = usePalette();
  const [picks, setPicks] = useState<Record<string, QuestionPick>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const answers = questionAnswers(ask.questions, picks);
  const update = (question: Question, change: (pick: QuestionPick) => QuestionPick) => {
    setError(null);
    setPicks((current) => ({ ...current, [question.id]: change(current[question.id] ?? NO_PICK) }));
  };
  const run = async (action: () => Promise<void>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await action();
      onDismiss();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const submit = () => answers && void run(() => onSubmit(answers));
  const skip = () => void run(onSkip);

  if (Platform.OS === "android") {
    return (
      <ComposeHost style={StyleSheet.absoluteFill} pointerEvents="box-none">
        <ModalBottomSheet onDismissRequest={onDismiss} containerColor={p.cell} contentColor={p.label}>
          <Column modifiers={[fillMaxWidth(), verticalScroll(), composePadding(0, 0, 0, 24)]}>
            {ask.questions.map((question) => (
              <ComposeQuestion key={question.id} question={question} pick={picks[question.id] ?? NO_PICK} onChange={(change) => update(question, change)} />
            ))}
            {error ? (
              <ComposeText color={p.red as string} style={{ typography: "bodySmall" }} modifiers={[composePadding(16, 8, 16, 0)]}>
                {error}
              </ComposeText>
            ) : null}
            <Row horizontalArrangement={{ spacedBy: 8 }} modifiers={[fillMaxWidth(), composePadding(16, 12, 16, 0)]}>
              <TextButton enabled={!busy} onClick={skip}>
                <ComposeText>{t("Skip")}</ComposeText>
              </TextButton>
              <ComposeButton enabled={!busy && answers != null} onClick={submit}>
                <ComposeText>{t("Submit")}</ComposeText>
              </ComposeButton>
            </Row>
          </Column>
        </ModalBottomSheet>
      </ComposeHost>
    );
  }

  return (
    <SwiftHost style={styles.host}>
      <BottomSheet isPresented onIsPresentedChange={(presented) => !presented && onDismiss()}>
        <Group modifiers={[presentationDragIndicator("visible"), presentationDetents(["medium", "large"])]}>
          <Form>
            {ask.questions.map((question) => (
              <SwiftQuestion key={question.id} question={question} pick={picks[question.id] ?? NO_PICK} tint={p.tint} onChange={(change) => update(question, change)} />
            ))}
            <Section footer={error ? <SwiftText modifiers={[font({ textStyle: "footnote" }), foregroundStyle("red")]}>{error}</SwiftText> : undefined}>
              <SwiftButton label={t("Submit")} onPress={submit} modifiers={[disabled(busy || answers == null)]} />
              <SwiftButton label={t("Skip")} onPress={skip} modifiers={[disabled(busy)]} />
            </Section>
          </Form>
        </Group>
      </BottomSheet>
    </SwiftHost>
  );
}

function SwiftQuestion({ question, pick, tint, onChange }: { question: Question; pick: QuestionPick; tint: ColorValue; onChange: (change: (pick: QuestionPick) => QuestionPick) => void }) {
  const other = useRef<TextFieldRef>(null);
  return (
    <Section title={question.header || undefined}>
      <SwiftText modifiers={[font({ textStyle: "headline" })]}>{question.question}</SwiftText>
      {question.options.map((option) => {
        const picked = pick.picked.includes(option.label);
        return (
          <SwiftButton
            key={option.label}
            onPress={() => {
              if (!question.multi_select && pick.other) void other.current?.clear();
              onChange(pickOption(question, option.label));
            }}
            modifiers={picked ? [accessibilityAddTraits(["isSelected"])] : undefined}
          >
            <HStack spacing={8}>
              <VStack alignment="leading" spacing={2}>
                <SwiftText modifiers={[foregroundStyle({ type: "hierarchical", style: "primary" })]}>{option.label}</SwiftText>
                {option.description ? <SwiftText modifiers={[font({ textStyle: "footnote" }), foregroundStyle({ type: "hierarchical", style: "secondary" })]}>{option.description}</SwiftText> : null}
              </VStack>
              <Spacer />
              {picked ? <SwiftImage systemName="checkmark" color={tint} /> : null}
            </HStack>
          </SwiftButton>
        );
      })}
      <TextField ref={other} placeholder={t("Other")} onTextChange={(text) => onChange(writeOther(question, text))} />
    </Section>
  );
}

function ComposeQuestion({ question, pick, onChange }: { question: Question; pick: QuestionPick; onChange: (change: (pick: QuestionPick) => QuestionPick) => void }) {
  const p = usePalette();
  const other = useRef<ComposeFieldRef>(null);
  return (
    <Column modifiers={[fillMaxWidth(), composePadding(0, 8, 0, 8)]}>
      {question.header ? (
        <ComposeText color={p.tint as string} style={{ typography: "labelLarge" }} modifiers={[composePadding(16, 4, 16, 4)]}>
          {question.header}
        </ComposeText>
      ) : null}
      <ComposeText style={{ typography: "titleMedium" }} modifiers={[composePadding(16, 4, 16, 4)]}>
        {question.question}
      </ComposeText>
      {question.options.map((option) => {
        const picked = pick.picked.includes(option.label);
        const tap = () => {
          if (!question.multi_select && pick.other) void other.current?.clear();
          onChange(pickOption(question, option.label));
        };
        return (
          <ListItem
            key={option.label}
            colors={{ containerColor: p.cell, contentColor: p.label, supportingContentColor: p.secondaryLabel }}
            modifiers={[fillMaxWidth(), selectable(picked, tap, question.multi_select ? "checkbox" : "radioButton")]}
          >
            <ListItem.LeadingContent>{question.multi_select ? <Checkbox value={picked} /> : <RadioButton selected={picked} />}</ListItem.LeadingContent>
            <ListItem.HeadlineContent>
              <ComposeText>{option.label}</ComposeText>
            </ListItem.HeadlineContent>
            {option.description ? (
              <ListItem.SupportingContent>
                <ComposeText>{option.description}</ComposeText>
              </ListItem.SupportingContent>
            ) : null}
          </ListItem>
        );
      })}
      <OutlinedTextField ref={other} singleLine onValueChange={(text) => onChange(writeOther(question, text))} modifiers={[fillMaxWidth(), composePadding(16, 4, 16, 0)]}>
        <OutlinedTextField.Placeholder>
          <ComposeText>{t("Other")}</ComposeText>
        </OutlinedTextField.Placeholder>
      </OutlinedTextField>
    </Column>
  );
}

const styles = StyleSheet.create({
  host: { position: "absolute", width: 0, height: 0 },
});
