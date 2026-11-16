#!/bin/bash

chars=`echo $((64*1024*1024))`
openssl rand -hex $chars


