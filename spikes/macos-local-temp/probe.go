package main

import (
	"fmt"
	"io"
	"net"
	"os"
)

func check(err error) {
	if err != nil {
		panic(err)
	}
}

func main() {
	for _, dir := range []string{"", "/tmp", "/private/tmp"} {
		file, err := os.CreateTemp(dir, "devcroft-local-temp-go-*")
		check(err)
		_, err = file.WriteString("go")
		check(err)
		check(file.Close())
		info, err := os.Stat(file.Name())
		check(err)
		if mode := info.Mode().Perm(); mode != 0600 {
			panic(fmt.Sprintf("%s has mode %o", file.Name(), mode))
		}
		data, err := os.ReadFile(file.Name())
		check(err)
		if string(data) != "go" {
			panic("unexpected temp file contents")
		}
		check(os.Remove(file.Name()))
	}

	socket := "/tmp/" + os.Getenv("DC_TAG") + ".sock"
	listener, err := net.Listen("unix", socket)
	check(err)
	defer listener.Close()
	client, err := net.Dial("unix", socket)
	check(err)
	defer client.Close()
	server, err := listener.Accept()
	check(err)
	defer server.Close()
	_, err = client.Write([]byte("go"))
	check(err)
	var data [2]byte
	_, err = io.ReadFull(server, data[:])
	check(err)
	if string(data[:]) != "go" {
		panic("unexpected socket contents")
	}
}
