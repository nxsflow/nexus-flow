#!/usr/bin/perl
# Throwaway: the live spawner that does NOT reap by itself (node would): own group by setpgid, as process_group(0) does.
use strict; use POSIX ":sys_wait_h"; use Errno;
my ($dir, $life, @cmd) = @ARGV;
open(my $L, ">>", "$dir/ladder.log") or die; select($L); $|=1;
sub st { my ($t,$p)=@_; my $r = kill(0,$p); my $e = $r ? "ok" : ($!{ESRCH} ? "ESRCH" : ($!{EPERM} ? "EPERM" : "$!")); print "$t: kill(0,$p)=$e\n"; }
sub group { my ($pg,$t)=@_; print "$t: ps of the group or marker:\n"; print map { "   $_" } grep { /^\s*\d+\s+\d+\s+$pg\s|TV58MARK/ } `ps -A -o pid=,ppid=,pgid=,stat=,command= | cut -c1-140`; }
my $pid = fork(); die unless defined $pid;
if (!$pid) { setpgrp(0,0); chdir $dir; open(STDOUT,">>","$dir/out.log"); open(STDERR,">&STDOUT"); exec(@cmd) or die; }
open(my $P, ">", "$dir/leader") or die; print $P "$pid\n"; close $P;
print "leader $pid\n"; 
my $reaped = 0; my $t0 = time;
sleep 1 while (time - $t0 < $life);
st("before", $pid); st("before", -$pid); group($pid,"before");
print "SIGTERM group\n"; kill('TERM', -$pid); 
for my $w (1,4,15) { sleep $w; st("term+$w unreaped", $pid); st("term+$w unreaped", -$pid); group($pid,"term+$w"); }
my $w = waitpid($pid, WNOHANG); print "waitpid WNOHANG -> $w status=$?\n"; $reaped = 1 if $w == $pid;
st("after reap", $pid); st("after reap", -$pid); group($pid,"after-reap");
print "second SIGTERM group\n"; kill('TERM', -$pid); sleep 5; st("term2", -$pid); group($pid,"term2");
print "SIGKILL group\n"; kill('KILL', -$pid); sleep 3; st("kill unreaped?", $pid); st("kill", -$pid); group($pid,"kill");
if (!$reaped) { $w = waitpid($pid, WNOHANG); print "waitpid WNOHANG -> $w\n"; st("after reap2", $pid); st("after reap2", -$pid); }
print "DONE\n";
