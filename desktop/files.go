package main

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"fmt"
	"image"
	_ "image/gif"
	_ "image/jpeg"
	_ "image/png"
	"io"
	"mime"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"time"

	"github.com/egoist/mygo"
	_ "golang.org/x/image/bmp"
	_ "golang.org/x/image/tiff"
	_ "golang.org/x/image/webp"
)

// fileScheme serves the files the pages show (attachments, bot images) by an opaque token, never
// by path: only a file the app handed out a URL for can be read.
const fileScheme = "lorca-file"

// FileInfo is a file on this computer as the composer and the bubbles need it.
type FileInfo struct {
	Path   string `json:"path"`
	Name   string `json:"name"`
	Size   int64  `json:"size"`
	Mime   string `json:"mime"`
	IsFile bool   `json:"isFile"`
	// Width and Height of an image as it shows upright, EXIF orientation applied.
	Width  *int `json:"width,omitempty"`
	Height *int `json:"height,omitempty"`
	// URL shows the file in a page.
	URL string `json:"url"`
}

// ChooseOptions configures Files.Choose.
type ChooseOptions struct {
	Title       string `json:"title,omitempty"`
	Message     string `json:"message,omitempty"`
	ButtonLabel string `json:"buttonLabel,omitempty"`
	Multiple    bool   `json:"multiple,omitempty"`
	// Images limits the choice to pictures.
	Images bool `json:"images,omitempty"`
}

type fileTokens struct {
	mu     sync.Mutex
	byPath map[string]string
	byID   map[string]string
}

var servedFiles = &fileTokens{byPath: map[string]string{}, byID: map[string]string{}}

// url is the address a page loads `path` from. Windows serves custom schemes from
// http://<scheme>.localhost, the others from <scheme>://localhost.
func (t *fileTokens) url(path string) string {
	t.mu.Lock()
	token, ok := t.byPath[path]
	if !ok {
		var raw [12]byte
		_, _ = rand.Read(raw[:])
		token = hex.EncodeToString(raw[:])
		t.byPath[path] = token
		t.byID[token] = path
	}
	t.mu.Unlock()
	name := url.PathEscape(filepath.Base(path))
	if runtime.GOOS == "windows" {
		return "http://" + fileScheme + ".localhost/" + token + "/" + name
	}
	return fileScheme + "://localhost/" + token + "/" + name
}

func (t *fileTokens) path(token string) (string, bool) {
	t.mu.Lock()
	defer t.mu.Unlock()
	path, ok := t.byID[token]
	return path, ok
}

func serveFile(w http.ResponseWriter, r *http.Request) {
	token, _, _ := strings.Cut(strings.TrimPrefix(r.URL.Path, "/"), "/")
	path, ok := servedFiles.path(token)
	if !ok {
		http.NotFound(w, r)
		return
	}
	file, err := os.Open(path)
	if err != nil {
		http.NotFound(w, r)
		return
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil || info.IsDir() {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Type", mimeType(path, file))
	w.Header().Set("Cache-Control", "no-cache")
	http.ServeContent(w, r, "", info.ModTime(), file)
}

// mimeType names a file's type by its extension, else by its first bytes.
func mimeType(path string, file io.ReadSeeker) string {
	if kind := mime.TypeByExtension(strings.ToLower(filepath.Ext(path))); kind != "" {
		kind, _, _ = strings.Cut(kind, ";")
		return kind
	}
	if file != nil {
		head := make([]byte, 512)
		n, _ := file.Read(head)
		_, _ = file.Seek(0, io.SeekStart)
		if n > 0 {
			kind, _, _ := strings.Cut(http.DetectContentType(head[:n]), ";")
			if kind != "text/plain" || bytes.IndexByte(head[:n], 0) < 0 {
				return kind
			}
		}
	}
	return "application/octet-stream"
}

// inspect describes a file for an attachment: what it is, how big, and an image's size.
func inspect(path string) FileInfo {
	info := FileInfo{Path: path, Name: filepath.Base(path), Mime: "application/octet-stream"}
	stat, err := os.Stat(path)
	if err != nil {
		return info
	}
	info.IsFile = stat.Mode().IsRegular()
	info.Size = stat.Size()
	if !info.IsFile {
		return info
	}
	file, err := os.Open(path)
	if err != nil {
		return info
	}
	defer file.Close()
	info.Mime = mimeType(path, file)
	if strings.HasPrefix(info.Mime, "image/") {
		if config, _, err := image.DecodeConfig(file); err == nil {
			width, height := config.Width, config.Height
			// A rotated photo (EXIF orientation 5–8) shows upright, so its box does too.
			if _, err := file.Seek(0, io.SeekStart); err == nil && exifOrientation(file) >= 5 {
				width, height = height, width
			}
			info.Width, info.Height = &width, &height
		}
	}
	info.URL = servedFiles.url(path)
	return info
}

// exifOrientation reads a JPEG's EXIF orientation tag, 1 when it has none.
func exifOrientation(r io.Reader) int {
	head := make([]byte, 64*1024)
	n, _ := io.ReadFull(r, head)
	data := head[:n]
	if len(data) < 4 || data[0] != 0xFF || data[1] != 0xD8 {
		return 1
	}
	for i := 2; i+4 <= len(data); {
		if data[i] != 0xFF {
			return 1
		}
		marker := data[i+1]
		length := int(binary.BigEndian.Uint16(data[i+2:]))
		segment := i + 4
		if marker == 0xE1 && segment+length-2 <= len(data) && bytes.HasPrefix(data[segment:], []byte("Exif\x00\x00")) {
			return tiffOrientation(data[segment+6 : segment+length-2])
		}
		if marker == 0xDA {
			return 1
		}
		i = segment + length - 2
	}
	return 1
}

func tiffOrientation(tiff []byte) int {
	if len(tiff) < 8 {
		return 1
	}
	var order binary.ByteOrder
	switch string(tiff[:2]) {
	case "II":
		order = binary.LittleEndian
	case "MM":
		order = binary.BigEndian
	default:
		return 1
	}
	offset := int(order.Uint32(tiff[4:]))
	if offset+2 > len(tiff) {
		return 1
	}
	count := int(order.Uint16(tiff[offset:]))
	for e := 0; e < count; e++ {
		entry := offset + 2 + e*12
		if entry+12 > len(tiff) {
			return 1
		}
		if order.Uint16(tiff[entry:]) == 0x0112 {
			value := int(order.Uint16(tiff[entry+8:]))
			if value >= 1 && value <= 8 {
				return value
			}
			return 1
		}
	}
	return 1
}

// Files reads and shows files on this computer for the pages.
type Files struct{}

// Choose asks for files, as the composer's + button and a bot's Look sheet do. It returns
// nothing when the user cancels.
func (Files) Choose(ctx context.Context, options ChooseOptions) ([]FileInfo, error) {
	var filters []mygo.FileFilter
	if options.Images {
		filters = []mygo.FileFilter{{Name: "Images", Extensions: []string{"png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"}}}
	}
	paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
		Parent:      mygo.CallerWindow(ctx),
		Title:       options.Title,
		Message:     options.Message,
		ButtonLabel: options.ButtonLabel,
		Multiple:    options.Multiple,
		Filters:     filters,
	})
	if err != nil {
		return nil, err
	}
	files := make([]FileInfo, 0, len(paths))
	for _, path := range paths {
		files = append(files, inspect(path))
	}
	return files, nil
}

// Inspect describes files dropped on a page.
func (Files) Inspect(paths []string) []FileInfo {
	files := make([]FileInfo, 0, len(paths))
	for _, path := range paths {
		files = append(files, inspect(path))
	}
	return files
}

// URL is where a page loads a file the CLI named, such as an attachment it fetched.
func (Files) URL(path string) string {
	return servedFiles.url(path)
}

// SavePasted writes what was pasted into the composer (a screenshot, a copied file) to a
// temporary file the CLI can read. An image with no name becomes "Pasted image <date>.png".
func (Files) SavePasted(name string, data []byte) (FileInfo, error) {
	directory := filepath.Join(os.TempDir(), "lorca-paste")
	if err := os.MkdirAll(directory, 0o700); err != nil {
		return FileInfo{}, err
	}
	name = filepath.Base(strings.TrimSpace(name))
	if name == "" || name == "." || name == string(filepath.Separator) || name == "image.png" {
		name = "Pasted image " + time.Now().Format("2006-01-02 at 15.04.05") + ".png"
	}
	path := filepath.Join(directory, name)
	if _, err := os.Stat(path); err == nil {
		ext := filepath.Ext(name)
		path = filepath.Join(directory, fmt.Sprintf("%s %d%s", strings.TrimSuffix(name, ext), time.Now().UnixNano(), ext))
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		return FileInfo{}, err
	}
	info := inspect(path)
	if !info.IsFile {
		return FileInfo{}, errors.New("the pasted file could not be saved")
	}
	return info, nil
}

// Open opens a file with its default app.
func (Files) Open(path string) error { return mygo.Shell.OpenPath(path) }

// ShowInFolder shows a file in the file manager.
func (Files) ShowInFolder(path string) { mygo.Shell.ShowItemInFolder(path) }
