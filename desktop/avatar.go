package main

import (
	"bytes"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"image"
	"image/png"
	"io"
	"math"
	"os"
	"path/filepath"
	"strings"
	"sync"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
	"golang.org/x/image/draw"
)

// Avatars: a bot's character (characters.go), its shape flat in its accent with two eyes, or its
// own image aspect-filled in a circle, the gray "you" and system discs, and the breathing green
// dot while the bot has a turn running, sitting on a ring of the background around it.

type avatarKind int

const (
	avatarBot avatarKind = iota
	avatarImage
	avatarYou
	avatarSystem
	// avatarContact is someone outside Lorca, by the first letter of their name.
	avatarContact
)

type avatarContent struct {
	Kind avatarKind
	// Shape is a bot's body, one of botShapes, and Key what shifts its motion while it works: the
	// bot's id.
	Shape  string
	Key    string
	Accent model.Accent
	// Path is an image's file on this computer.
	Path string
	// Name is a contact's name.
	Name string
}

// botAvatar is the bot's image when this computer has the bytes (the store fetches them and
// redraws otherwise), else its character.
func botAvatar(bot *model.Bot) avatarContent {
	if path := store.AvatarPath(bot); path != "" {
		if bitmap := loadBitmap(path); bitmap != nil {
			return avatarContent{Kind: avatarImage, Path: path}
		}
	}
	return avatarContent{Kind: avatarBot, Shape: shapeFor(bot.Shape, bot.ID), Key: bot.ID, Accent: bot.Accent}
}

// templateAvatar is the character a marketplace bot starts with; one without a shape shows the
// one its id picks, standing in for the id the new bot gets.
func templateAvatar(template model.BotTemplate) avatarContent {
	return avatarContent{Kind: avatarBot, Shape: shapeFor(template.Shape, template.ID), Key: template.ID, Accent: template.Accent}
}

func authorAvatar(author model.Author) avatarContent {
	switch author.Kind {
	case model.AuthorYou:
		return avatarContent{Kind: avatarYou}
	case model.AuthorContact:
		return avatarContent{Kind: avatarContact, Name: author.Name}
	case model.AuthorBot:
		if bot := store.Bot(author.BotID); bot != nil {
			return botAvatar(bot)
		}
	}
	return avatarContent{Kind: avatarSystem}
}

// presencePlace is where the working dot sits: over the bottom-right edge of a circle `size` across,
// or the top-right one when a service's mark holds the bottom.
func presencePlace(size float32, top bool) (dot, x, y float32) {
	dot = float32(math.Max(7, math.Round(float64(size)*0.28)))
	if top {
		return dot, size - dot + 1, -1
	}
	return dot, size - dot + 1, size - dot + 1
}

// drawPresence breathes the green working dot over `box`, a circle `size` across, on a ring of
// `backdrop`.
func drawPresence(c *ui.Context, box ui.Element, size float32, top bool, backdrop ui.Color) {
	green := colors(c).Green
	box.DrawOver(func(painter *ui.Painter, r ui.Rect) {
		painter.AnimationFrame()
		dot, x, y := presencePlace(size, top)
		cx, cy := r.X+x+dot/2, r.Y+y+dot/2
		// Breathing: 1 to 0.8 and back, 1.2 seconds each way.
		phase := float64(painter.Now().UnixMilli()%2400) / 1200
		if phase > 1 {
			phase = 2 - phase
		}
		eased := float32(0.5 - 0.5*math.Cos(phase*math.Pi))
		var ring, disc ui.Path
		ring.Circle(cx, cy, dot/2+2)
		painter.FillPath(&ring, backdrop)
		disc.Circle(cx, cy, dot/2*(1-0.2*eased))
		painter.FillPath(&disc, green)
	})
}

// avatar is one avatar, `size` across. While `working`, the green dot breathes on a ring of the
// content's background.
func avatar(c *ui.Context, content avatarContent, size float32, working bool) ui.Element {
	return avatarOn(c, content, size, working, colors(c).Content)
}

// avatarOn is an avatar whose working dot rings itself in `backdrop`, the color behind the avatar.
func avatarOn(c *ui.Context, content avatarContent, size float32, working bool, backdrop ui.Color) ui.Element {
	return avatarRinged(c, content, size, working, backdrop, 0)
}

// avatarRinged is avatarOn standing off what is behind it by `ring` of `backdrop`: around a
// character's outline, or around the circle.
func avatarRinged(c *ui.Context, content avatarContent, size float32, working bool, backdrop ui.Color, ring float32) ui.Element {
	p := colors(c)
	box := ui.Box(c).Size(size, size)
	if content.Kind == avatarBot {
		// A working character acts out its clips beside the dot (motion.go); at rest it stands still.
		body := p.accentColor(content.Accent)
		moving := working && !c.Preferences().ReduceMotion
		key, seed := content.Key, motionSeed(content.Key)
		box.Draw(func(painter *ui.Painter, r ui.Rect) {
			drawCharacter(painter, r, content.Shape, body, ring, backdrop, animate(painter, moving, key, seed))
		})
		if working {
			drawPresence(c, box, size, false, backdrop)
		}
		return box
	}
	if ring > 0 {
		box.Radius(size/2).Shadow(0, 0, 0, ring, backdrop)
	}
	box.Children(func() {
		disc := ui.Box(c).Size(size, size).Radius(size / 2).Clip().Center().TextColor(p.White)
		switch content.Kind {
		case avatarYou:
			disc.Background(p.Label3)
		case avatarContact:
			gray := ui.RGB(142, 142, 147)
			disc.Gradient(gray.Mix(p.White, 0.25), gray, 180)
		default:
			disc.Background(p.Label4)
		}
		disc.Children(func() {
			switch content.Kind {
			case avatarImage:
				if bitmap := loadBitmap(content.Path); bitmap != nil {
					ui.Image(c, bitmap).Size(size, size).Fit(ui.Cover)
				}
			case avatarContact:
				initial := "?"
				if name := []rune(strings.TrimSpace(content.Name)); len(name) > 0 {
					initial = strings.ToUpper(string(name[0]))
				}
				ui.Text(c, initial).FontSize(float32(math.Round(float64(size * 0.46)))).FontWeight(600).SingleLine()
			default:
				name := "gearshape.fill"
				if content.Kind == avatarYou {
					name = "person.fill"
				}
				stroke := float32(2.4)
				if size >= 40 {
					stroke = 2
				}
				symbol(c, name, float32(math.Round(float64(size*0.5))), stroke)
			}
		})
	})
	if working {
		drawPresence(c, box, size, false, backdrop)
	}
	return box
}

type clusterBox struct{ x, y, size float32 }

// avatarCluster packs group avatars into a fixed square: a row of overlapping circles would grow
// with the member count and push the title along, and a constant slot keeps every row's text in
// line. A channel conversation's `service` (telegram, slack) puts its mark on the front avatar's
// bottom-right edge and sends the working dot to the top-right.
func avatarCluster(c *ui.Context, contents []avatarContent, slot float32, working bool, service string, backdrop ui.Color) ui.Element {
	count := min(len(contents), 4)
	var boxes []clusterBox
	size := float32(math.Round(float64(slot) * 0.6))
	free := slot - size
	switch {
	case count <= 1:
		boxes = []clusterBox{{1, 1, slot - 2}}
	case count == 2:
		boxes = []clusterBox{{0, 0, size}, {free, free, size}}
	case count == 3:
		boxes = []clusterBox{{0, 0, size}, {free, 0, size}, {free / 2, free, size}}
	default:
		boxes = []clusterBox{{0, 0, size}, {free, 0, size}, {0, free, size}, {free, free, size}}
	}
	cluster := ui.Box(c).Size(slot, slot)
	cluster.Children(func() {
		for i, b := range boxes {
			if i >= len(contents) {
				break
			}
			cell := ui.Box(c).Absolute().Left(b.x).Top(b.y).Size(b.size, b.size)
			// The front avatars stand off those behind by a ring of the backdrop.
			ring := float32(0)
			if i > 0 {
				ring = 1
			}
			front := i == len(boxes)-1
			marked := front && service != ""
			cell.Children(func() {
				avatarRinged(c, contents[i], b.size, working && front && !marked, backdrop, ring)
				if mark := pluginMark(service); marked && mark != nil {
					serviceBadge(c, mark, service, b.size, backdrop)
				}
			})
			if marked && working {
				drawPresence(c, cell, b.size, true, backdrop)
			}
		}
	})
	return cluster
}

// serviceBadge is a service's mark over the bottom-right edge of an avatar `size` across:
// Telegram's disc as it is, any other mark on a white disc, ringed in the backdrop.
func serviceBadge(c *ui.Context, mark *ui.SVG, service string, size float32, backdrop ui.Color) {
	badge := float32(math.Round(float64(size) * 0.42))
	inset := float32(0)
	if service != "telegram" {
		inset = float32(math.Round(float64(badge) * 0.22))
	}
	ui.Box(c).Absolute().Left(size-badge+1).Top(size-badge+1).Size(badge, badge).Radius(badge/2).
		Background(colors(c).White).Shadow(0, 0, 0, 1.5, backdrop).Padding(inset).
		Children(func() { ui.Image(c, mark).Fill() })
}

// avatarStack is overlapping avatars where there is room to spread out: up to three, later ones
// on top, each ringed in the backdrop.
func avatarStack(c *ui.Context, bots []*model.Bot, size, overlap float32, backdrop ui.Color) ui.Element {
	shown := bots[:min(3, len(bots))]
	width := size + float32(max(0, len(shown)-1))*(size-overlap)
	stack := ui.Box(c).Size(width, size)
	stack.Children(func() {
		for i, bot := range shown {
			cell := ui.Box(c).Absolute().Top(0).Left(float32(i)*(size-overlap)).Size(size, size)
			ring := float32(0)
			if len(shown) > 1 {
				ring = 1.5
			}
			cell.Children(func() { avatarRinged(c, botAvatar(bot), size, false, backdrop, ring) })
		}
	})
	return stack
}

// MARK: - Images

type bitmapEntry struct {
	bitmap  *ui.Bitmap
	loading bool
	failed  bool
}

var bitmaps = struct {
	sync.Mutex
	entries map[string]*bitmapEntry
}{entries: map[string]*bitmapEntry{}}

// loadBitmap is the picture at `path` once it is read: the first ask starts reading it off the main
// thread and redraws the windows when it lands; until then, and when it cannot be read, it is nil.
func loadBitmap(path string) *ui.Bitmap {
	if path == "" {
		return nil
	}
	bitmaps.Lock()
	entry, ok := bitmaps.entries[path]
	if !ok {
		entry = &bitmapEntry{loading: true}
		bitmaps.entries[path] = entry
	}
	bitmaps.Unlock()
	if ok {
		return entry.bitmap
	}
	go func() {
		data, err := os.ReadFile(path)
		var bitmap *ui.Bitmap
		if err == nil {
			bitmap, err = ui.DecodeBitmap(data)
		}
		mygo.RunOnMain(func() {
			bitmaps.Lock()
			entry.loading = false
			entry.bitmap = bitmap
			entry.failed = err != nil
			bitmaps.Unlock()
			invalidateWindows()
		})
	}()
	return nil
}

// MARK: - Profile images

// avatarSide is the longest side of a stored profile image: small enough to sync in a moment,
// large enough for the biggest avatar any screen draws.
const avatarSide = 512

var errNotImage = errors.New("that file could not be read as an image")

// prepareAvatar makes a bot's profile image from a picture on this computer: a square, upright,
// center-cropped PNG of at most 512 px, written to a temporary file the CLI copies into its store.
// The bytes that leave the computer are these, not the original.
func prepareAvatar(path string) (model.AvatarFile, error) {
	file, err := os.Open(path)
	if err != nil {
		return model.AvatarFile{}, errNotImage
	}
	defer file.Close()
	source, _, err := image.Decode(file)
	if err != nil {
		return model.AvatarFile{}, errNotImage
	}
	orientation := 1
	if _, err := file.Seek(0, io.SeekStart); err == nil {
		orientation = exifOrientation(file)
	}
	bounds := source.Bounds()
	width, height := bounds.Dx(), bounds.Dy()
	if width <= 0 || height <= 0 {
		return model.AvatarFile{}, errNotImage
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
	var out bytes.Buffer
	if err := png.Encode(&out, upright(square, orientation)); err != nil {
		return model.AvatarFile{}, err
	}
	if err := os.WriteFile(target, out.Bytes(), 0o600); err != nil {
		return model.AvatarFile{}, err
	}
	return model.AvatarFile{Path: target, Name: filepath.Base(path), Mime: "image/png", Size: int64(out.Len())}, nil
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
