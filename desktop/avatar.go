package main

import (
	"crypto/rand"
	"encoding/hex"
	"errors"
	"image"
	"image/png"
	"io"
	"os"
	"path/filepath"

	"golang.org/x/image/draw"
)

// avatarSide is the longest side of a stored profile image: small enough to sync in a moment,
// large enough for the biggest avatar any screen draws.
const avatarSide = 512

var errNotImage = errors.New("that file could not be read as an image")

// PrepareAvatar makes a bot's profile image from a picture on this computer: a square, upright,
// center-cropped PNG of at most 512 px, written to a temporary file the CLI copies into its store.
// The bytes that leave the computer are these, not the original.
func (Files) PrepareAvatar(path string) (FileInfo, error) {
	file, err := os.Open(path)
	if err != nil {
		return FileInfo{}, errNotImage
	}
	defer file.Close()
	source, _, err := image.Decode(file)
	if err != nil {
		return FileInfo{}, errNotImage
	}
	orientation := 1
	if _, err := file.Seek(0, io.SeekStart); err == nil {
		orientation = exifOrientation(file)
	}
	bounds := source.Bounds()
	width, height := bounds.Dx(), bounds.Dy()
	if width <= 0 || height <= 0 {
		return FileInfo{}, errNotImage
	}

	// Aspect-fill the square from the middle of the picture.
	short := min(width, height)
	side := min(avatarSide, short)
	crop := image.Rect(0, 0, short, short).Add(bounds.Min).Add(image.Pt((width-short)/2, (height-short)/2))
	square := image.NewNRGBA(image.Rect(0, 0, side, side))
	draw.CatmullRom.Scale(square, square.Bounds(), source, crop, draw.Src, nil)

	var raw [4]byte
	_, _ = rand.Read(raw[:])
	target := filepath.Join(os.TempDir(), "lorca-avatar-"+hex.EncodeToString(raw[:])+".png")
	out, err := os.OpenFile(target, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o600)
	if err != nil {
		return FileInfo{}, err
	}
	if err := png.Encode(out, upright(square, orientation)); err != nil {
		out.Close()
		os.Remove(target)
		return FileInfo{}, err
	}
	if err := out.Close(); err != nil {
		return FileInfo{}, err
	}
	return inspect(target), nil
}

// upright turns a square image the way its EXIF orientation says it shows.
func upright(square *image.NRGBA, orientation int) *image.NRGBA {
	if orientation <= 1 || orientation > 8 {
		return square
	}
	n := square.Bounds().Dx()
	last := n - 1
	turned := image.NewNRGBA(square.Bounds())
	for y := range n {
		for x := range n {
			var sx, sy int
			switch orientation {
			case 2: // mirrored
				sx, sy = last-x, y
			case 3: // upside down
				sx, sy = last-x, last-y
			case 4: // mirrored upside down
				sx, sy = x, last-y
			case 5: // mirrored, turned left
				sx, sy = y, x
			case 6: // turned right
				sx, sy = y, last-x
			case 7: // mirrored, turned right
				sx, sy = last-y, last-x
			case 8: // turned left
				sx, sy = last-y, x
			}
			turned.SetNRGBA(x, y, square.NRGBAAt(sx, sy))
		}
	}
	return turned
}
