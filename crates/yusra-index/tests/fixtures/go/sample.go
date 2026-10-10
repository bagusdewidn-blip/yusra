//go:build linux

// Package sample adalah contoh untuk uji ekstraksi simbol. Tidak dikompilasi.
package sample

import "fmt"

import (
	"errors"
	nethttp "net/http"
	_ "embed"
	`strings`
)

// MaxRetry adalah batas percobaan.
const MaxRetry = 3

const (
	// StateActive menandai session aktif.
	StateActive = iota
	StateClosed
	first, second = 1, 2
)

var ErrNotFound = errors.New("tidak ditemukan")

type Status int

// Session adalah session login.
type Session struct {
	ID    int
	token string
}

type (
	// Store menyimpan session.
	Store interface {
		Get(id int) (*Session, error)
	}

	Cache[T any] struct {
		items map[int]T
	}
)

// MakeToken membuat token.
func MakeToken(id int) string {
	return fmt.Sprintf("t%d", id)
}

// Refresh memperbarui token.
func (s *Session) Refresh() bool {
	s.token = MakeToken(s.ID)
	return strings.HasPrefix(s.token, "t")
}

func (s Session) String() string {
	return fmt.Sprint(s.ID)
}

func (c *Cache[T]) Put(id int, v T) {
	c.items[id] = v
}

// komentar ini terpisah baris kosong

func handler(w nethttp.ResponseWriter, r *nethttp.Request) {
	fmt.Fprintln(w, "ok")
}
