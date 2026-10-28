package main

import (
	"math"
	"slices"

	"github.com/egoist/mygo/ui"
)

// Bot characters, after Grok Bot's: a flat body in the bot's accent, one of a dozen shapes, with
// two white eyes glancing up and to the right. The outlines are made here, once, and the same
// numbers draw the phone app's masks and the website's paths (characters_test.go writes them).

// botShapes are the bodies, in the order `crates/cli/src/model.rs` lists them as BOT_SHAPES.
var botShapes = []string{
	"blob", "pebble", "squircle", "cloud", "drop", "sparkle", "clover", "flower", "heart", "hex", "star", "cushion",
}

// shapeFor is the bot's shape: the one it was given, or one its id picks (FNV-1a), so bots told
// apart by nothing else still look different.
func shapeFor(shape, id string) string {
	if slices.Contains(botShapes, shape) {
		return shape
	}
	hash := uint32(2166136261)
	for i := 0; i < len(id); i++ {
		hash ^= uint32(id[i])
		hash *= 16777619
	}
	return botShapes[hash%uint32(len(botShapes))]
}

// character is a body's outline and its eyes, in a box from -1 to 1 with y down.
type character struct {
	outline [][2]float64
	// eyes are each eye's two ends; an eye is a line between them drawn eyeWidth wide with round caps.
	eyes [2][2][2]float64
}

const (
	// outlinePoints is how finely an outline is sampled, every 2°.
	outlinePoints = 180
	// eyeWidth is an eye's width in the box, a twelfth of the body.
	eyeWidth = 0.17
	// eyeReach is how far an eye's ends sit from its middle.
	eyeReach = 0.13
	// eyeTilt leans the eyes' tops to the left, in radians.
	eyeTilt = 25 * math.Pi / 180
	// bodyReach is the body's half extent: it fills the box but for a sliver.
	bodyReach = 0.95
)

// characters are made once, by shape.
var characters = func() map[string]*character {
	made := map[string]*character{}
	for _, shape := range botShapes {
		made[shape] = makeCharacter(shape)
	}
	return made
}()

func makeCharacter(shape string) *character {
	spec := shapeSpecs[shape]
	radii := make([]float64, outlinePoints)
	for i := range radii {
		radii[i] = spec.radius(2 * math.Pi * float64(i) / outlinePoints)
	}
	radii = smooth(radii, spec.smoothing)
	points := make([][2]float64, outlinePoints)
	sin, cos := math.Sincos(spec.tilt * math.Pi / 180)
	for i, r := range radii {
		s, c := math.Sincos(2 * math.Pi * float64(i) / outlinePoints)
		x, y := r*c, r*s
		points[i] = [2]float64{x*cos - y*sin, x*sin + y*cos}
	}
	// Center the body's bounds and scale its longer side to the box.
	minX, minY, maxX, maxY := math.Inf(1), math.Inf(1), math.Inf(-1), math.Inf(-1)
	for _, p := range points {
		minX, maxX = min(minX, p[0]), max(maxX, p[0])
		minY, maxY = min(minY, p[1]), max(maxY, p[1])
	}
	scale := 2 * bodyReach / max(maxX-minX, maxY-minY)
	midX, midY := (minX+maxX)/2, (minY+maxY)/2
	for i, p := range points {
		points[i] = [2]float64{(p[0] - midX) * scale, (p[1] - midY) * scale}
	}
	made := &character{outline: points}
	dx, dy := eyeReach*math.Sin(eyeTilt), eyeReach*math.Cos(eyeTilt)
	for i, at := range [2][2]float64{{0.04, -0.2}, {0.44, -0.3}} {
		x, y := at[0]+spec.eyes[0], at[1]+spec.eyes[1]
		made.eyes[i] = [2][2]float64{{x - dx, y - dy}, {x + dx, y + dy}}
	}
	return made
}

// shapeSpec makes a body as a radius by angle around its middle: every shape is one the middle
// sees all of, so a ray from it crosses the edge once.
type shapeSpec struct {
	radius func(angle float64) float64
	// smoothing averages the radius over this many degrees each way, rounding corners and the
	// creases between lobes.
	smoothing float64
	// tilt turns the body clockwise, in degrees.
	tilt float64
	// eyes moves the eyes from where they sit on most bodies.
	eyes [2]float64
}

var shapeSpecs = map[string]shapeSpec{
	"blob": {radius: func(a float64) float64 {
		return 1 + 0.05*math.Sin(2*a+0.6) + 0.04*math.Sin(3*a+1.9)
	}},
	"pebble":   {radius: ellipse(1, 0.8), tilt: -14},
	"squircle": {radius: superellipse(0.9, 1, 4), tilt: 10},
	"cloud": {radius: circles([][3]float64{
		{-0.5, 0.2, 0.38}, {-0.15, -0.12, 0.48}, {0.36, -0.2, 0.42}, {0.62, 0.18, 0.34}, {0.14, 0.3, 0.38}, {-0.22, 0.32, 0.36},
	}), smoothing: 3, tilt: -8, eyes: [2]float64{-0.02, 0.04}},
	"drop":    {radius: polygon(teardrop()), smoothing: 8, tilt: -40, eyes: [2]float64{0.02, 0.06}},
	"sparkle": {radius: polygon(star(4, 1, 0.38, -math.Pi/2)), smoothing: 11, tilt: 8, eyes: [2]float64{-0.22, 0.24}},
	"clover": {radius: circles([][3]float64{
		{0.5, 0, 0.48}, {-0.5, 0, 0.48}, {0, 0.5, 0.48}, {0, -0.5, 0.48},
	}), smoothing: 4, tilt: 45, eyes: [2]float64{-0.06, 0.1}},
	"flower":  {radius: petals(6, 0.56, 0.38), smoothing: 8, tilt: 15, eyes: [2]float64{-0.08, 0.1}},
	"heart":   {radius: polygon(heart()), smoothing: 10, eyes: [2]float64{-0.12, 0.2}},
	"hex":     {radius: polygon(star(6, 1, 1, 0)), smoothing: 14, tilt: 12},
	"star":    {radius: polygon(star(5, 1, 0.56, -math.Pi/2)), smoothing: 16, tilt: 6, eyes: [2]float64{-0.24, 0.24}},
	"cushion": {radius: superellipse(1, 1, 6), smoothing: 4, tilt: -16},
}

// smooth averages each radius over its neighbors within `degrees` either way, around the ring.
func smooth(radii []float64, degrees float64) []float64 {
	reach := int(math.Round(degrees / 360 * float64(len(radii))))
	if reach == 0 {
		return radii
	}
	smoothed := make([]float64, len(radii))
	for i := range radii {
		var sum float64
		for k := -reach; k <= reach; k++ {
			sum += radii[(i+k+len(radii))%len(radii)]
		}
		smoothed[i] = sum / float64(2*reach+1)
	}
	return smoothed
}

func ellipse(a, b float64) func(float64) float64 {
	return func(angle float64) float64 {
		s, c := math.Sincos(angle)
		return a * b / math.Hypot(b*c, a*s)
	}
}

func superellipse(a, b, n float64) func(float64) float64 {
	return func(angle float64) float64 {
		s, c := math.Sincos(angle)
		return math.Pow(math.Pow(math.Abs(c)/a, n)+math.Pow(math.Abs(s)/b, n), -1/n)
	}
}

// circles is the far edge of the circles {x, y, r} a ray from the middle crosses.
func circles(list [][3]float64) func(float64) float64 {
	return func(angle float64) float64 {
		s, c := math.Sincos(angle)
		var far float64
		for _, circle := range list {
			// |t·(c, s) − center|² = r²; the larger root.
			along := c*circle[0] + s*circle[1]
			d := along*along - (circle[0]*circle[0] + circle[1]*circle[1] - circle[2]*circle[2])
			if d >= 0 {
				far = max(far, along+math.Sqrt(d))
			}
		}
		return far
	}
}

// petals is `count` circles of radius `r` around the middle at `distance`, over a disc that fills
// the gaps between them.
func petals(count int, distance, r float64) func(float64) float64 {
	list := [][3]float64{{0, 0, distance}}
	for i := range count {
		s, c := math.Sincos(2 * math.Pi * float64(i) / float64(count))
		list = append(list, [3]float64{distance * c, distance * s, r})
	}
	return circles(list)
}

// polygon is where a ray from the middle leaves a polygon the middle sees all of.
func polygon(points [][2]float64) func(float64) float64 {
	return func(angle float64) float64 {
		s, c := math.Sincos(angle)
		var far float64
		for i, a := range points {
			b := points[(i+1)%len(points)]
			// Solve t·(c, s) = a + u·(b − a) for t ≥ 0 and u in [0, 1], a hair wider so a ray
			// through a corner meets an edge whichever way the rounding goes.
			ex, ey := b[0]-a[0], b[1]-a[1]
			den := c*ey - s*ex
			if math.Abs(den) < 1e-12 {
				continue
			}
			t := (a[0]*ey - a[1]*ex) / den
			u := (a[0]*s - a[1]*c) / den
			if t > 0 && u >= -1e-9 && u <= 1+1e-9 {
				far = max(far, t)
			}
		}
		return far
	}
}

// star alternates `count` outer points with inner ones; inner = outer is a regular polygon.
func star(count int, outer, inner, start float64) [][2]float64 {
	var points [][2]float64
	for i := range 2 * count {
		r := outer
		if i%2 == 1 {
			r = inner
		}
		if inner == outer && i%2 == 1 {
			continue
		}
		s, c := math.Sincos(start + math.Pi*float64(i)/float64(count))
		points = append(points, [2]float64{r * c, r * s})
	}
	return points
}

// heart is the classic heart curve, centered on the middle of its height.
func heart() [][2]float64 {
	var points [][2]float64
	for i := range 120 {
		t := 2 * math.Pi * float64(i) / 120
		x := 16 * math.Pow(math.Sin(t), 3)
		y := -(13*math.Cos(t) - 5*math.Cos(2*t) - 2*math.Cos(3*t) - math.Cos(4*t))
		points = append(points, [2]float64{x / 16, (y + 2.5) / 16})
	}
	return points
}

// teardrop points up from a round bottom.
func teardrop() [][2]float64 {
	var points [][2]float64
	for i := range 120 {
		t := 2 * math.Pi * float64(i) / 120
		x := math.Sin(t) * math.Pow(math.Sin(t/2), 1.4)
		y := -math.Cos(t)
		points = append(points, [2]float64{x, y - 0.25})
	}
	return points
}

// MARK: - Drawing

// The head the body is drawn as: what a turn does to the outline and to the eyes.
const (
	// headDepth is how far the eyes stand out of the body's plane: the body is a puffy disc, not a
	// ball, so a turn slides them across the face without carrying them past its edge.
	headDepth = 0.6
)

// animate is the pose to draw now: a working character's, asking the window for the next frame,
// or the still one at rest and when the desktop asks for less motion.
func animate(painter *ui.Painter, moving bool, key string, seed uint32) pose {
	if !moving {
		return restPose
	}
	painter.AnimationFrame()
	now := painter.Now()
	return workingPose(now.Sub(workStart(key, now)).Seconds(), seed)
}

// bodyPoint maps a point of the box into `r` for the body in `at`: squeezed as a puffy disc is
// seen turned and tipped, scaled, leaned, and moved.
func bodyPoint(r ui.Rect, at pose, p [2]float64) (float32, float32) {
	yaw, pitch := at.yaw*math.Pi/180, at.pitch*math.Pi/180
	x := p[0] * at.scale * (0.8 + 0.2*math.Abs(math.Cos(yaw)))
	y := p[1] * at.scale * (0.6 + 0.4*math.Abs(math.Cos(pitch)))
	return lean(r, at, x, y)
}

// lean turns a point by the pose's roll, moves it, and maps it into `r`.
func lean(r ui.Rect, at pose, x, y float64) (float32, float32) {
	sin, cos := math.Sincos(at.roll * math.Pi / 180)
	x, y = x*cos-y*sin+at.x, x*sin+y*cos+at.y
	half := float64(r.W) / 2
	return r.X + float32((1+x)*half), r.Y + float32(float64(r.H)/2+y*half)
}

// eyeAt is where an eye's middle shows for a character in `at`, and how much of it faces the
// viewer: 1 straight on, 0 when it has turned away.
func eyeAt(made *character, at pose, i int) (x, y, facing float64) {
	eyes := made.eyes
	mid := func(e [2][2]float64) (float64, float64) { return (e[0][0] + e[1][0]) / 2, (e[0][1] + e[1][1]) / 2 }
	hx, hy := mid(eyes[i])
	ax, ay := mid(eyes[0])
	bx, by := mid(eyes[1])
	// Centered, the pair keeps its spacing about the middle of the face, a little high.
	cx, cy := hx-(ax+bx)/2, hy-(ay+by)/2-0.1
	x, y = hx+(cx-hx)*at.face.center, hy+(cy-hy)*at.face.center
	z := headDepth * math.Sqrt(max(0.25, 1-x*x-y*y))
	yaw, pitch := at.yaw*math.Pi/180, at.pitch*math.Pi/180
	x, z = x*math.Cos(yaw)+z*math.Sin(yaw), -x*math.Sin(yaw)+z*math.Cos(yaw)
	y, z = y*math.Cos(pitch)-z*math.Sin(pitch), y*math.Sin(pitch)+z*math.Cos(pitch)
	// Past about 70° off straight on, the eye has gone round the edge.
	return x * at.scale, y * at.scale, math.Min(1, math.Max(0, (z/headDepth-0.35)/0.4))
}

// characterPath is the body's outline in `r`.
func characterPath(c *character, r ui.Rect, at pose) *ui.Path {
	var path ui.Path
	for i, p := range c.outline {
		x, y := bodyPoint(r, at, p)
		if i == 0 {
			path.MoveTo(x, y)
		} else {
			path.LineTo(x, y)
		}
	}
	path.Close()
	return &path
}

// drawCharacter paints the shape's body in `body` with white eyes in `r`, a square, standing in
// `at`. A `ring` wider than 0 first paints the body's outline that much wider in `backdrop`, to
// stand it off what is behind.
func drawCharacter(painter *ui.Painter, r ui.Rect, shape string, body ui.Color, ring float32, backdrop ui.Color, at pose) {
	made := characters[shape]
	if made == nil {
		return
	}
	path := characterPath(made, r, at)
	if ring > 0 {
		painter.StrokePath(path, ring*2, backdrop)
	}
	painter.FillPath(path, body)
	// A blink draws each eye in to its middle, down to a dot; one turned away shrinks out of sight.
	half := float64(r.W) / 2
	for i := range made.eyes {
		x, y, facing := eyeAt(made, at, i)
		if facing <= 0 {
			continue
		}
		size := facing * at.scale
		reach := (at.face.length - at.face.width) / 2 * (1 - at.lids) * size
		width := at.face.width * (1 - 0.3*at.lids) * size
		sin, cos := math.Sincos(at.face.tilt * math.Pi / 180)
		dx, dy := reach*sin, reach*cos
		var line ui.Path
		line.MoveTo(lean(r, at, x-dx, y-dy))
		line.LineTo(lean(r, at, x+dx, y+dy))
		painter.StrokePath(&line, float32(width*half), ui.RGB(255, 255, 255))
	}
}

// characterView is a box `size` across that draws the character at rest.
func characterView(c *ui.Context, shape string, body ui.Color, size float32) ui.Element {
	box := ui.Box(c).Size(size, size)
	box.Draw(func(painter *ui.Painter, r ui.Rect) {
		drawCharacter(painter, r, shape, body, 0, body, restPose)
	})
	return box
}
