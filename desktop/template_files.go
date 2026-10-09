package main

import (
	"net/url"
	"os"
	"path/filepath"
	"strings"

	"github.com/egoist/mygo"
)

// The system's Open and Save dialogs for template files, which tests replace. They answer a path;
// the CLI reads and writes the file.
var chooseTemplateSource = openTemplateSource
var chooseTemplateDestination = saveTemplateDestination

// pendingTemplateLink is a shared bot's link the app was opened with, kept until there is an
// account and a CLI to read it: Open in Lorca on lorca.app may be what started the app.
var pendingTemplateLink string

// openTemplateLink takes lorca://t/<id>#<key> (lorca-dev:// for Lorca Dev) from a shared bot's
// page, which the sheet shows as the page's own address.
func (a *appDelegate) openTemplateLink(link string) {
	page, err := url.Parse(link)
	if err != nil || page.Host != "t" {
		return
	}
	page.Scheme, page.Host, page.Path = "https", "lorca.app", "/t"+page.Path
	pendingTemplateLink = page.String()
	a.presentPendingTemplateLink()
}

func (a *appDelegate) presentPendingTemplateLink() {
	if pendingTemplateLink == "" || store.HasIdentity == nil || !*store.HasIdentity || !(store.IsConnected || store.IsMock) {
		return
	}
	link := pendingTemplateLink
	pendingTemplateLink = ""
	a.showMainWindow()
	if a.main != nil {
		a.main.presentTemplateImport(link, "", a.main.selectChat)
	}
}

// copyText puts a shared link on the clipboard; tests keep it off the user's.
var copyText = func(text string) { mygo.Clipboard.WriteText(text) }

func openTemplateSource(parent *mygo.Window, done func(string, error)) {
	options := mygo.OpenDialogOptions{Parent: parent, Filters: []mygo.FileFilter{{Name: L("Bot templates"), Extensions: []string{"lorca-template"}}}}
	go func() {
		paths, err := mygo.Dialog.Open(options)
		path := ""
		if len(paths) > 0 {
			path = paths[0]
		}
		post(func() { done(path, err) })
	}()
}

func saveTemplateDestination(parent *mygo.Window, name string, done func(string, error)) {
	options := mygo.SaveDialogOptions{Parent: parent, DefaultPath: templateFilename(name), Filters: []mygo.FileFilter{{Name: L("Bot templates"), Extensions: []string{"lorca-template"}}}}
	go func() {
		path, err := mygo.Dialog.Save(options)
		post(func() { done(path, err) })
	}()
}

func templateFilename(name string) string {
	name = strings.Map(func(r rune) rune {
		if r < 32 || strings.ContainsRune(`<>:"/\|?*`, r) {
			return '-'
		}
		return r
	}, name)
	name = strings.Trim(name, " .")
	if name == "" {
		name = "bot"
	}
	stem := strings.ToUpper(strings.SplitN(name, ".", 2)[0])
	if stem == "CON" || stem == "PRN" || stem == "AUX" || stem == "NUL" ||
		(len(stem) == 4 && (strings.HasPrefix(stem, "COM") || strings.HasPrefix(stem, "LPT")) && stem[3] >= '1' && stem[3] <= '9') {
		name = "bot-" + name
	}
	return name + ".lorca-template"
}

// Native Save confirms an existing path. If adding the required extension selects a different
// existing path, the sheet explicitly asks before replacing that file too.
func templateDestination(path string) (normalized string, exists, needsConfirmation bool) {
	normalized = path
	if !strings.HasSuffix(path, ".lorca-template") {
		if strings.EqualFold(filepath.Ext(path), ".lorca-template") {
			normalized = strings.TrimSuffix(path, filepath.Ext(path)) + ".lorca-template"
		} else {
			normalized += ".lorca-template"
		}
	}
	_, err := os.Lstat(normalized)
	exists = err == nil
	return normalized, exists, exists && normalized != path
}
