package model

import (
	"fmt"
	"math"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/l10n"
)

// Dates, sizes, and schedules in the app's language, after the macOS app's `Format`. English keeps
// a 12-hour clock, as the United States does, and Chinese a 24-hour one, as China does.

// Now is the clock the formatters read; tests set it.
var Now = time.Now

func startOfDay(t time.Time) time.Time {
	y, m, d := t.Date()
	return time.Date(y, m, d, 0, 0, 0, 0, t.Location())
}

// daysFromToday is calendar days from today: 0 today, -1 yesterday, 1 tomorrow. Rounding absorbs
// the 23- and 25-hour days of a clock change.
func daysFromToday(t time.Time) int {
	return int(math.Round(startOfDay(t.Local()).Sub(startOfDay(Now().Local())).Hours() / 24))
}

func IsToday(t time.Time) bool     { return daysFromToday(t) == 0 }
func IsYesterday(t time.Time) bool { return daysFromToday(t) == -1 }
func isTomorrow(t time.Time) bool  { return daysFromToday(t) == 1 }

func IsSameDay(a, b time.Time) bool { return startOfDay(a.Local()).Equal(startOfDay(b.Local())) }

var (
	weekdaysZH      = []string{"星期日", "星期一", "星期二", "星期三", "星期四", "星期五", "星期六"}
	shortWeekdaysZH = []string{"周日", "周一", "周二", "周三", "周四", "周五", "周六"}
)

// Clock is "4:13 PM", or "16:13" in Chinese.
func Clock(t time.Time) string {
	t = t.Local()
	if l10n.IsChinese() {
		return t.Format("15:04")
	}
	return t.Format("3:04 PM")
}

func weekday(t time.Time) string {
	if l10n.IsChinese() {
		return weekdaysZH[t.Weekday()]
	}
	return t.Weekday().String()
}

// monthDay is "Sep 3", or "9月3日".
func monthDay(t time.Time) string {
	if l10n.IsChinese() {
		return fmt.Sprintf("%d月%d日", t.Month(), t.Day())
	}
	return t.Format("Jan 2")
}

// Stamp is the stamp for sidebar rows: the time today, "Yesterday", the weekday within a week,
// "9/2" this year, "9/2/25" before that.
func Stamp(t time.Time) string {
	t = t.Local()
	switch {
	case IsToday(t):
		return Clock(t)
	case IsYesterday(t):
		return L("Yesterday")
	case Now().Sub(t) < 7*24*time.Hour:
		return weekday(t)
	case t.Year() == Now().Year():
		return fmt.Sprintf("%d/%d", t.Month(), t.Day())
	}
	if l10n.IsChinese() {
		return fmt.Sprintf("%02d/%d/%d", t.Year()%100, t.Month(), t.Day())
	}
	return fmt.Sprintf("%d/%d/%02d", t.Month(), t.Day(), t.Year()%100)
}

// Once is a one-time routine's date and time on its own clock: "Once on Oct 12 at 9:00 AM", with
// the year when it is not this one.
func Once(at time.Time, timezone string) string {
	if zone, err := time.LoadLocation(timezone); err == nil {
		at = at.In(zone)
	}
	day := at.Format("Jan 2")
	clock := at.Format("3:04 PM")
	if l10n.IsChinese() {
		day, clock = fmt.Sprintf("%d月%d日", at.Month(), at.Day()), at.Format("15:04")
	}
	if at.Year() != Now().Year() {
		if l10n.IsChinese() {
			day = fmt.Sprintf("%d年", at.Year()) + day
		} else {
			day += at.Format(", 2006")
		}
	}
	return L("Once on %@ at %@", day, clock)
}

// AroundEvents is a time around calendar events: "15 minutes before each event", "When events
// matching “Customer” end".
func AroundEvents(minutes int, after bool, matching string) string {
	span := L("%d minutes", minutes)
	switch {
	case minutes == 60:
		span = L("1 hour")
	case minutes > 60 && minutes%60 == 0:
		span = L("%d hours", minutes/60)
	case minutes == 1:
		span = L("1 minute")
	}
	switch {
	case matching == "" && minutes == 0 && !after:
		return L("When each event starts")
	case matching == "" && minutes == 0:
		return L("When each event ends")
	case matching == "" && !after:
		return L("%@ before each event", span)
	case matching == "":
		return L("%@ after each event ends", span)
	case minutes == 0 && !after:
		return L("When events matching “%@” start", matching)
	case minutes == 0:
		return L("When events matching “%@” end", matching)
	case !after:
		return L("%@ before events matching “%@”", span, matching)
	}
	return L("%@ after events matching “%@” end", span, matching)
}

// DaySeparator is the separator before a cluster of messages: "Today 4:13 AM", "Yesterday
// 9:55 AM", "Thu, Sep 10, 9:48 AM".
func DaySeparator(t time.Time) string {
	t = t.Local()
	switch {
	case IsToday(t):
		return L("Today %@", Clock(t))
	case IsYesterday(t):
		return L("Yesterday %@", Clock(t))
	}
	if l10n.IsChinese() {
		return monthDay(t) + shortWeekdaysZH[t.Weekday()] + " " + Clock(t)
	}
	return t.Format("Mon, Jan 2, ") + Clock(t)
}

// LastSeen is when an offline Device was last on the relay.
func LastSeen(t time.Time) string {
	elapsed := Now().Sub(t).Seconds()
	switch {
	case elapsed < 60:
		return L("Last seen just now")
	case elapsed < 3600:
		return L("Last seen %dm ago", int(elapsed/60))
	case elapsed < 86400:
		return L("Last seen %dh ago", int(elapsed/3600))
	}
	return L("Last seen %dd ago", int(elapsed/86400))
}

// Upcoming is "today 9:00 AM", "tomorrow 9:00 AM", "Monday 9:00 AM", "Oct 1 9:00 AM": when a run
// is due.
func Upcoming(t time.Time) string {
	t = t.Local()
	switch {
	case IsToday(t):
		return L("today %@", Clock(t))
	case isTomorrow(t):
		return L("tomorrow %@", Clock(t))
	case t.Sub(Now()) < 6*24*time.Hour:
		return weekday(t) + " " + Clock(t)
	}
	return monthDay(t) + " " + Clock(t)
}

// DateTime is "Today 9:00 AM" or "Sep 3, 2026, 9:00 AM": a date with its time, for "Last checked".
func DateTime(t time.Time) string {
	t = t.Local()
	switch {
	case IsToday(t):
		return L("Today %@", Clock(t))
	case IsYesterday(t):
		return L("Yesterday %@", Clock(t))
	}
	if l10n.IsChinese() {
		return fmt.Sprintf("%d年%d月%d日 %s", t.Year(), t.Month(), t.Day(), Clock(t))
	}
	return t.Format("Jan 2, 2006, ") + Clock(t)
}

// Kilobytes is "1.2 KB", "24 KB".
func Kilobytes(bytes int) string {
	kb := float64(bytes) / 1000
	if kb >= 10 || kb == math.Round(kb) {
		return fmt.Sprintf("%d KB", int(math.Round(kb)))
	}
	return fmt.Sprintf("%.1f KB", kb)
}

// Tokens is "950", "12k", "1.2M".
func Tokens(count int) string {
	switch {
	case count >= 1_000_000:
		return fmt.Sprintf("%.1fM", float64(count)/1_000_000)
	case count >= 1_000:
		return fmt.Sprintf("%dk", count/1_000)
	}
	return strconv.Itoa(count)
}

// Elapsed is "0:42", "12:03", "1:02:03".
func Elapsed(from, now time.Time) string {
	seconds := max(0, int(now.Sub(from).Seconds()))
	hours, minutes, rest := seconds/3600, seconds%3600/60, seconds%60
	if hours > 0 {
		return fmt.Sprintf("%d:%02d:%02d", hours, minutes, rest)
	}
	return fmt.Sprintf("%d:%02d", minutes, rest)
}

var (
	scheduleSingle   = regexp.MustCompile(`^Every (day|hour|minute)$`)
	scheduleInterval = regexp.MustCompile(`^Every (\d+) (day|hour|minute)s$`)
	schedulePast     = regexp.MustCompile(`^Every hour at :(\d{2})$`)
	scheduleClock    = regexp.MustCompile(`^(\d{1,2}):(\d{2}) (AM|PM)$`)
	scheduleMonthly  = regexp.MustCompile(`^On the (.+) of every month$`)
	leadingDigits    = regexp.MustCompile(`^\d+`)
)

// Schedule is a schedule in the app's language. The CLI words it in English ("Weekdays at 9:00 AM
// and 5:30 PM", "Every 2 hours", "On the 1st and 15th of every month at 9:00 AM"); in Chinese the
// same sentence is rebuilt from its parts. A sentence that is not one of the CLI's reads as it came.
func Schedule(text string) string {
	if !l10n.IsChinese() {
		return text
	}
	list := func(words string) []string {
		words = strings.ReplaceAll(words, ", and ", ", ")
		words = strings.ReplaceAll(words, " and ", ", ")
		return strings.Split(words, ", ")
	}
	clock := func(word string) string {
		parts := scheduleClock.FindStringSubmatch(word)
		if parts == nil {
			return word
		}
		half := "上午"
		if parts[3] == "PM" {
			half = "下午"
		}
		return half + " " + parts[1] + ":" + parts[2]
	}
	units := map[string]string{"day": "天", "hour": "小时", "minute": "分钟"}
	if m := scheduleSingle.FindStringSubmatch(text); m != nil {
		return "每" + units[m[1]]
	}
	if m := scheduleInterval.FindStringSubmatch(text); m != nil {
		return "每 " + m[1] + " " + units[m[2]]
	}
	if m := schedulePast.FindStringSubmatch(text); m != nil {
		n, _ := strconv.Atoi(m[1])
		return fmt.Sprintf("每小时的第 %d 分", n)
	}
	at := strings.LastIndex(text, " at ")
	if at < 0 {
		return text
	}
	var times []string
	for _, word := range list(text[at+4:]) {
		times = append(times, clock(word))
	}
	joined := strings.Join(times, "、")
	days := text[:at]
	switch days {
	case "Every day":
		return "每天 " + joined
	case "Weekdays":
		return "工作日 " + joined
	case "Weekends":
		return "周末 " + joined
	}
	if m := scheduleMonthly.FindStringSubmatch(days); m != nil {
		var dates []string
		for _, word := range list(m[1]) {
			n, _ := strconv.Atoi(leadingDigits.FindString(word))
			dates = append(dates, fmt.Sprintf("%d 日", n))
		}
		return "每月 " + strings.Join(dates, "、") + " " + joined
	}
	weekdays := map[string]string{"Sunday": "周日", "Monday": "周一", "Tuesday": "周二", "Wednesday": "周三", "Thursday": "周四", "Friday": "周五", "Saturday": "周六"}
	var names []string
	for _, day := range list(strings.TrimPrefix(days, "Every ")) {
		name, ok := weekdays[day]
		if !ok {
			return text
		}
		names = append(names, name)
	}
	return "每" + strings.Join(names, "、") + " " + joined
}
