#!/bin/env python3

from subprocess import call
from struct import *
import sys
import os
import hashlib


def main():
    args = sys.argv[1::]
    if len(args) < 5:
        print(sys.argv[0] + " in out prefix key iv")
        return

input_file = sys.argv[1]
output_file = sys.argv[2]
identical_in_out = output_file == input_file

key = sys.argv[4]
iv = sys.argv[5]
sha256 = hashlib.sha256()

BUF_SIZE = 65536  # lets read stuff in 64kb chunks!


with open(input_file, 'rb') as f:
    while True:
        data = f.read(BUF_SIZE)
        if not data:
            break
        sha256.update(data)
print("sha256: {0}".format(sha256.hexdigest()))

if (identical_in_out):
    output_file = output_file + ".ident"

# Encrypt
call(["openssl", "enc", "-aes-256-cbc", "-bufsize", "16", "-in", input_file, "-out", output_file, "-K", key, "-iv", iv])

# Prepand data
out=open(output_file, 'rb')
otmp=open(output_file + ".tmp", 'w+b')

# Prefix
otmp.write(sys.argv[3].ljust(16, '0'))

# IV
otmp.write(bytearray.fromhex(iv))

# Original hash
otmp.write(sha256.digest())


while True:
    data = out.read(BUF_SIZE)
    if not data:
        break
    otmp.write(data)

out.close()
otmp.close()
os.remove(output_file)
os.rename(output_file + ".tmp", output_file)

if (identical_in_out):
    os.rename(input_file, input_file + "_plain")
    os.rename(output_file, input_file)
