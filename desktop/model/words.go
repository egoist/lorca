package model

import "github.com/egoist/lorca/desktop/l10n"

// L is the app's words in the user's language: `L("New Chat")`, `L("%@ is working…", name)`.
func L(key string, args ...any) string { return l10n.L(key, args...) }

// Lc is one English word that means two things, looked up with its context.
func Lc(text, context string) string { return l10n.Lc(text, context) }
