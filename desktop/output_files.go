package main

import (
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"unicode/utf8"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// Native file handlers are separate from view builds so no context/element reaches a worker.
var outputOpenPath = mygo.Shell.OpenPath
var outputSaveDestination = func(parent *mygo.Window, name string) (string, error) {
	return mygo.Dialog.Save(mygo.SaveDialogOptions{Parent: parent, Title: L("Save As…"), ButtonLabel: L("Save"), DefaultPath: filepath.Base(name)})
}

func (s *outputsState) openFile(id, path string) {
	action := s.actions[id]
	action.busy, action.errorText = true, ""
	model.Async(store, func() (struct{}, error) { return struct{}{}, outputOpenPath(path) }, func(_ struct{}, err error) {
		if !s.active() {
			return
		}
		action.busy, action.errorText = false, model.ErrorText(err)
		s.sheet.window.invalidate()
	})
}

func (s *outputsState) saveFile(id string, attachment model.Attachment, source string) {
	action := s.actions[id]
	action.busy, action.errorText = true, ""
	parent := s.sheet.window.win
	model.Async(store, func() (struct{}, error) {
		target, err := outputSaveDestination(parent, attachment.Name)
		if err == nil && target != "" {
			err = copyOutput(source, target)
		}
		return struct{}{}, err
	}, func(_ struct{}, err error) {
		if !s.active() {
			return
		}
		action.busy, action.errorText = false, model.ErrorText(err)
		s.sheet.window.invalidate()
	})
}

// copyOutput reads only the CLI-provided local file. A download/save never follows a document
// link or fetches a Runner filesystem path. Write errors leave an existing destination intact.
func copyOutput(source, target string) error {
	input, err := os.Open(source)
	if err != nil {
		return err
	}
	defer input.Close()
	sourceInfo, err := input.Stat()
	if err != nil {
		return err
	}
	if !sourceInfo.Mode().IsRegular() {
		return fmt.Errorf("%s", L("File unavailable"))
	}
	if info, err := os.Stat(target); err == nil && os.SameFile(sourceInfo, info) {
		return nil
	}
	temporary, err := os.CreateTemp(filepath.Dir(target), ".lorca-output-*")
	if err != nil {
		return err
	}
	tmp := temporary.Name()
	defer os.Remove(tmp)
	n, err := io.Copy(temporary, io.LimitReader(input, 100*1024*1024+1))
	if err == nil && n > 100*1024*1024 {
		err = fmt.Errorf("%s", L("Output exceeds the file size limit"))
	}
	if err == nil {
		err = temporary.Sync()
	}
	closeErr := temporary.Close()
	if err != nil {
		return err
	}
	if closeErr != nil {
		return closeErr
	}
	return os.Rename(tmp, target)
}

type outputPreview struct {
	text      string
	bitmap    *ui.Bitmap
	errorText string
	loading   bool
}

func (w *appWindow) presentOutputPreview(attachment model.Attachment, path string) {
	state := &outputPreview{loading: true}
	var owner *sheet
	owner = w.present(func(c *ui.Context, s *sheet) {
		p := colors(c)
		result := sheetFrame(c, sheetOptions{Title: attachment.Name, Subtitle: attachment.Mime + " · " + model.SizeText(attachment.Size), Width: 640, Confirm: L("Done"), NoCancel: true, Leading: func() {
			if pushButton(c.Key("open-preview"), L("Open"), pushOptions{Disabled: state.loading}).Clicked() {
				go outputOpenPath(path)
			}
		}}, func() {
			switch {
			case state.loading:
				ui.Text(c, L("Loading preview…")).FontSize(12).TextColor(p.Label2)
			case state.errorText != "":
				ui.Text(c, L("Preview unavailable: %@", state.errorText)).FontSize(12).TextColor(p.Red)
			case state.bitmap != nil:
				ui.Image(c, state.bitmap).Width(560).MaxWidthPercent(100).Height(320).Fit(ui.Contain)
			case state.text != "":
				markdownView(c, state.text, markdownOptions{})
			default:
				ui.Text(c, L("Open this file in its system app to preview it.")).FontSize(12).TextColor(p.Label2)
			}
		})
		if result.Confirmed || result.Cancelled {
			s.dismiss()
		}
	}, nil)
	model.Async(store, func() (outputPreview, error) {
		result := outputPreview{}
		file, err := os.Open(path)
		if err != nil {
			return result, err
		}
		defer file.Close()
		if attachment.IsImage() {
			data, err := io.ReadAll(io.LimitReader(file, 100*1024*1024+1))
			if err != nil {
				return result, err
			}
			if len(data) > 100*1024*1024 {
				return result, fmt.Errorf("%s", L("Output exceeds the file size limit"))
			}
			result.bitmap, err = ui.DecodeBitmap(data)
			return result, err
		}
		if outputTextPreview(attachment.Mime) {
			data, err := io.ReadAll(io.LimitReader(file, 64*1024+1))
			if err != nil {
				return result, err
			}
			if !utf8.Valid(data) {
				return result, fmt.Errorf("%s", L("This file has no UTF-8 text preview"))
			}
			text := string(data)
			if len(data) > 64*1024 {
				text = strings.ToValidUTF8(string(data[:64*1024]), "") + "\n\n" + L("Preview truncated; open or save the full file.")
			}
			// Code fences display logs/JSON as data rather than interpreting their Markdown links.
			fence := "```"
			for strings.Contains(text, fence) {
				fence += "`"
			}
			result.text = fence + "\n" + text + "\n" + fence
		}
		return result, nil
	}, func(result outputPreview, err error) {
		if owner.window == nil {
			return
		}
		result.errorText = model.ErrorText(err)
		*state = result
		owner.window.invalidate()
	})
}
