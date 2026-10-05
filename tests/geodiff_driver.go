package main
import("fmt";"os";"github.com/tinyowl-labs/go-geodiff/geodiff")
func main(){var err error;switch os.Args[1]{case "diff":err=geodiff.CreateChangeset(os.Args[2],os.Args[3],os.Args[4]);case "apply":err=geodiff.ApplyChangeset(os.Args[2],os.Args[3]);default:err=fmt.Errorf("unknown operation")};if err!=nil{fmt.Fprintln(os.Stderr,err);os.Exit(1)}}
