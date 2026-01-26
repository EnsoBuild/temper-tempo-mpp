// Temper Adapter - Auto-loads simulation on page load

// The main loadSimulation function is defined in script.js
// This file just handles auto-loading when SIMULATION_ID is set

document.addEventListener('DOMContentLoaded', function() {
    if (window.SIMULATION_ID && typeof window.loadSimulation === 'function') {
        window.loadSimulation(window.SIMULATION_ID);
    }
});
