// Mounts the harness: the module's own stylesheet and primitives, on a page of their own.
import '../../../src/ds/styles/system.css';
import './preview.css';
import { mount } from 'svelte';
import Harness from './Harness.svelte';

mount(Harness, { target: document.getElementById('app') });
