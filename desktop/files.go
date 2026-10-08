package main

import (
	"bytes"
	"encoding/binary"
	"errors"
	"fmt"
	"image"
	_ "image/gif"
	_ "image/jpeg"
	_ "image/png"
	"io"
	"mime"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/egoist/mygo"
	_ "golang.org/x/image/bmp"
	_ "golang.org/x/image/tiff"
	_ "golang.org/x/image/webp"
)

// fileInfo is a file on this computer as the composer and the bubbles need it.
type fileInfo struct {
	Path   string
	Name   string
	Size   int64
	Mime   string
	IsFile bool
	// Width and Height of an image as it shows upright, EXIF orientation applied; 0 when unknown.
	Width  int
	Height int
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
func inspect(path string) fileInfo {
	info := fileInfo{Path: path, Name: filepath.Base(path), Mime: "application/octet-stream"}
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
			info.Width, info.Height = width, height
		}
	}
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

// chooseFiles asks for files, as the composer's + button and a bot's Look sheet do, and answers
// them on the main thread: none when the user cancels.
func chooseFiles(parent *mygo.Window, title, message, button string, multiple, images bool, done func([]fileInfo)) {
	var filters []mygo.FileFilter
	if images {
		filters = []mygo.FileFilter{{Name: L("Images"), Extensions: []string{"png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"}}}
	}
	go func() {
		paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{
			Parent:      parent,
			Title:       title,
			Message:     message,
			ButtonLabel: button,
			Multiple:    multiple,
			Filters:     filters,
		})
		var files []fileInfo
		if err == nil {
			for _, path := range paths {
				files = append(files, inspect(path))
			}
		}
		post(func() { done(files) })
	}()
}

// savePasted writes what was pasted into the composer (a screenshot, a copied file) to a temporary
// file the CLI can read. An image with no name becomes "Pasted image <date>.png".
func savePasted(name string, data []byte) (fileInfo, error) {
	directory := filepath.Join(os.TempDir(), "lorca-paste")
	if err := os.MkdirAll(directory, 0o700); err != nil {
		return fileInfo{}, err
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
		return fileInfo{}, err
	}
	info := inspect(path)
	if !info.IsFile {
		return fileInfo{}, errors.New("the pasted file could not be saved")
	}
	return info, nil
}

// openFile opens a file with its default app.
func openFile(path string) { go mygo.Shell.OpenPath(path) }

// showInFolder shows a file in the file manager.
func showInFolder(path string) { go mygo.Shell.ShowItemInFolder(path) }
